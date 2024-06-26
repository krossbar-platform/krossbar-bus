use std::{
    fmt::{self, Debug, Formatter},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use futures::stream::FusedStream;
use krossbar_bus_common::protocols::hub::{Message as HubMessage, HUB_CONNECT_METHOD};
use log::{debug, error, info, warn};
use serde::{de::DeserializeOwned, Serialize};

use crate::event::Event;
use krossbar_rpc::{request::RpcRequest, rpc::Rpc, writer::RpcWriter};

pub type Stream<T> = Pin<Box<dyn FusedStream<Item = crate::Result<T>> + Send>>;

/// Service connection handle
#[derive(Clone)]
pub struct Client {
    /// Peer service name
    service_name: String,
    /// Peer writer to send messages
    writer: RpcWriter,
    /// Hub reconnect event to listen if hub is disconnected
    hub_reconnect_event: Event<crate::Result<()>>,
    /// If the connection is outgoing. If it is we try to reconnect to
    /// the service if connection dropped
    outgoing: Arc<AtomicBool>,
}

impl Client {
    pub(crate) fn new(
        service_name: String,
        writer: RpcWriter,
        hub_reconnect_event: Event<crate::Result<()>>,
        outgoing: Arc<AtomicBool>,
    ) -> Self {
        Self {
            service_name,
            writer,
            hub_reconnect_event,
            outgoing,
        }
    }

    pub(crate) fn make_outgoing(&mut self) {
        self.outgoing.store(true, Ordering::Relaxed)
    }

    /// Make a method call. Returns an error immediately if data can't be serialized into [bson::Bson] or
    /// connection is down. If sends request succesfully, waits for a response.
    /// Tries to deserialize the response into `R`. Returns an error if types are incompatible.
    pub async fn call<P: Serialize, R: DeserializeOwned>(
        &self,
        endpoint: &str,
        params: &P,
    ) -> crate::Result<R> {
        match self.writer.call(endpoint, params).await {
            Ok(data) => data.await,
            // Client disconnected. Wait for main loop to reconnect
            Err(_) => {
                info!(
                    "Client {} disconnected. Waiting to reconnect",
                    self.service_name
                );

                if let Err(e) = self.hub_reconnect_event.wait().await {
                    Err(e)
                } else {
                    Box::pin(self.call(endpoint, params)).await
                }
            }
        }
    }

    /// Get a remote state value. Equals to `call::<R, ()>(endpoint, &())`, which means
    /// you can use it for any method with unit params type.
    pub async fn get<R: DeserializeOwned>(&self, endpoint: &str) -> crate::Result<R> {
        self.call(endpoint, &()).await
    }

    /// Send a one-way message. Equals to a `call` w/o expecting the call result
    pub async fn message<P: Serialize>(&self, endpoint: &str, body: &P) -> crate::Result<()> {
        match self.writer.send_message(endpoint, body).await {
            Ok(data) => Ok(data),
            // Client disconnected. Wait for main loop to reconnect
            Err(_) => {
                if let Err(e) = self.hub_reconnect_event.wait().await {
                    Err(e)
                } else {
                    Box::pin(self.message(endpoint, body)).await
                }
            }
        }
    }

    /// Subscribe to a signal of a state.
    /// Returns an error immediately if connection is down.
    /// Returns a stream of signal emissions.
    /// Tries to deserialize the response into `R`. Returns [None] if failes and stops handling the subscription.
    pub async fn subscribe<R: DeserializeOwned>(&self, endpoint: &str) -> crate::Result<Stream<R>> {
        match self.writer.subscribe(endpoint).await {
            Ok(stream) => Ok(stream),
            // Client disconnected. Wait for main loop to reconnect
            Err(e) => {
                warn!(
                    "Subscription failed as client disconnected: {}. Waiting to reconnect",
                    e.to_string()
                );

                if let Err(e) = self.hub_reconnect_event.wait().await {
                    Err(e)
                } else {
                    Box::pin(self.subscribe(endpoint)).await
                }
            }
        }
    }
}

impl Debug for Client {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "Client: {}", self.service_name)
    }
}

pub(crate) enum ClientEvent {
    Message(String, RpcRequest),
    Disconnect(String),
}
pub(crate) struct ClientHandle {
    service_name: String,
    rpc: Rpc,
    hub_connection: RpcWriter,
    hub_reconnect_event: Event<crate::Result<()>>,
    outgoing: Arc<AtomicBool>,
    self_reconnect_event: Event<crate::Result<()>>,
    wait_connect: bool,
}

impl ClientHandle {
    pub fn new(
        service_name: String,
        rpc: Rpc,
        hub_connection: RpcWriter,
        hub_reconnect_event: Event<crate::Result<()>>,
        wait_connect: bool,
    ) -> Self {
        Self {
            service_name,
            rpc,
            hub_connection,
            hub_reconnect_event,
            outgoing: Arc::new(false.into()),
            self_reconnect_event: Event::new(),
            wait_connect,
        }
    }

    pub async fn connect(
        service_name: &str,
        hub_connection: RpcWriter,
        hub_reconnect_event: Event<crate::Result<()>>,
        wait_connect: bool,
    ) -> crate::Result<Self> {
        let service_name = service_name.to_owned();

        let rpc = Self::hub_connect(
            &hub_connection,
            &hub_reconnect_event,
            &service_name,
            wait_connect,
        )
        .await?;

        Ok(Self {
            service_name,
            rpc,
            hub_connection,
            hub_reconnect_event,
            outgoing: Arc::new(true.into()),
            self_reconnect_event: Event::new(),
            wait_connect,
        })
    }

    pub fn client_handle(&self) -> Client {
        Client::new(
            self.service_name.clone(),
            self.rpc.writer().clone(),
            self.self_reconnect_event.clone(),
            self.outgoing.clone(),
        )
    }

    pub async fn poll(&mut self) -> ClientEvent {
        debug!("Client {} poll", self.service_name);

        match self.rpc.poll().await {
            Some(request) => ClientEvent::Message(self.service_name.clone(), request),
            None => {
                // Client disconnected
                if self.outgoing.load(Ordering::Relaxed) {
                    warn!("Outgoing connection lost. Trying to reconnect");

                    // Failing handshake means we've lost permissions to connect
                    match Self::hub_connect(
                        &self.hub_connection,
                        &self.hub_reconnect_event,
                        &self.service_name,
                        self.wait_connect,
                    )
                    .await
                    {
                        Ok(rpc) => {
                            self.rpc.on_reconnected(rpc).await;

                            self.self_reconnect_event.emit(Ok(())).await;
                            Box::pin(self.poll()).await
                        }
                        Err(_) => {
                            warn!("Hub rejected reconnect request");

                            self.self_reconnect_event
                                .emit(Err(crate::Error::PeerDisconnected))
                                .await;
                            ClientEvent::Disconnect(self.service_name.clone())
                        }
                    }
                } else {
                    info!("Incoming connection lost");

                    self.self_reconnect_event
                        .emit(Err(crate::Error::PeerDisconnected))
                        .await;
                    ClientEvent::Disconnect(self.service_name.clone())
                }
            }
        }
    }

    async fn hub_connect(
        hub_connection: &RpcWriter,
        hub_reconnect_event: &Event<crate::Result<()>>,
        service_name: &String,
        wait: bool,
    ) -> crate::Result<Rpc> {
        loop {
            info!("Connecting to the service: {service_name}");

            match hub_connection
                .call_fd::<HubMessage, ()>(
                    HUB_CONNECT_METHOD,
                    &HubMessage::Connect {
                        service_name: service_name.clone(),
                        wait,
                    },
                )
                .await
            {
                Ok(future) => match future.await {
                    Ok((_, stream)) => return Ok(Rpc::new(stream, service_name)),
                    Err(e) => {
                        error!("Hub error during peer connection: {e:?}");
                        return Err(e);
                    }
                },
                Err(e) => {
                    warn!(
                        "Failed to reconnect to a client: {e:?}. Hub is down. Waiting to reconnect"
                    );

                    hub_reconnect_event.wait().await?
                }
            }
        }
    }
}
