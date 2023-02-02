use std::{
    fmt::Debug,
    sync::{atomic::AtomicBool, Arc},
};

use anyhow::{Context, Result};
use bson::Bson;
use karo_common_rpc::{rpc_connection::RpcConnection, rpc_sender::RpcSender};
use log::*;
use serde::{de::DeserializeOwned, Serialize};
use tokio::{
    net::UnixStream,
    sync::{
        mpsc::{self, Receiver, Sender},
        Mutex,
    },
};
use tokio_stream::{Stream, StreamExt};

use karo_bus_common::{
    errors::Error as BusError,
    monitor::{MonitorMessage, MonitorMessageDirection},
};

use karo_common_messages::{Message as MessageBody, Response};
use karo_common_rpc::Message;

use crate::peer_connector::PeerConnector;

/// P2p service connection handle
#[derive(Clone)]
pub struct Peer {
    /// Own service name
    service_name: String,
    /// Peer service name
    peer_service_name: String,
    /// We reconnect only to the outgoing peer connections. This is passed to the connector.
    /// This value can change, because we can start subscribing to a service, which
    /// previously initiated connection.
    outgoing: Arc<AtomicBool>,
    /// Sender into the socket to return to the client
    peer_sender: RpcSender,
    /// Sender to shutdown peer connection
    shutdown_tx: Sender<()>,
    /// Monitor connection
    monitor: Arc<Mutex<Option<RpcSender>>>,
}

impl Peer {
    /// Create new service handle and start tokio task to handle incoming messages from the peer
    /// *incoming_stream* If passed, this is an incoming connection, if None, peer handle should
    ///     connect itself
    /// *hub_writer* Sends a message directrly to the hub. Used for reconnection
    pub(crate) async fn new(
        service_name: String,
        peer_service_name: String,
        incoming_stream: Option<UnixStream>,
        interface_tx: Sender<(RpcSender, Message)>,
        hub_writer: RpcSender,
    ) -> Result<Self> {
        // If we have an incoming stream, we don't reconnect
        let outgoing: Arc<AtomicBool> = Arc::new(if incoming_stream.is_none() {
            true.into()
        } else {
            false.into()
        });

        info!(
            "Registered new {} connection from {}",
            if incoming_stream.is_none() {
                "outgoing"
            } else {
                "incoming"
            },
            peer_service_name
        );

        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

        // Peer connector, which will connect to the peer if this is and outgoing connection
        let connector = Box::new(PeerConnector::new(
            peer_service_name.clone(),
            hub_writer,
            incoming_stream,
            outgoing.clone(),
        ));

        // Rpc connection
        let rpc_connection = RpcConnection::new(connector).await?;

        let peer_sender = rpc_connection.sender();

        let monitor = Arc::new(Mutex::new(None));

        Self::start_task(
            rpc_connection,
            monitor.clone(),
            shutdown_rx,
            interface_tx,
            service_name.clone(),
            peer_service_name.clone(),
        );

        Ok(Self {
            service_name: service_name.clone(),
            peer_service_name: peer_service_name.clone(),
            outgoing,
            peer_sender,
            shutdown_tx,
            monitor,
        })
    }

    async fn start_task(
        mut rpc_connection: RpcConnection,
        monitor: Arc<Mutex<Option<RpcSender>>>,
        mut shutdown_rx: Receiver<()>,
        interface_tx: Sender<(RpcSender, Message)>,
        service_name: String,
        peer_service_name: String,
    ) {
        let mut peer_sender = rpc_connection.sender();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    // Read incoming message from the peer. RpcConnection::read return only if we've got
                    // an incoming message, not a response.
                    // If disconnected, we receive Response::Shutdowm, and after that start
                    // sending None to exclude peer handle from polling. Also, this message is sent when a peer
                    // shutting down gracefully
                    incoming_message = rpc_connection.read() => {
                        match incoming_message {
                            Ok(message) => {
                                let internal_message = message.body();

                                Self::send_monitor_message(&monitor,
                                    &internal_message,
                                    MonitorMessageDirection::Incoming,
                                    &service_name,
                                    &peer_service_name).await;

                                if matches!(internal_message, MessageBody::Response(Response::Shutdown(_))) {
                                    warn!("Peer connection closed. Shutting him down");
                                    return;
                                } else {
                                    // This is an incoming message. Send it to the interfaces
                                    if interface_tx.send((peer_sender.clone(), message)).await.is_err() {
                                        warn!("Peer connection closed. Shutting down");
                                        return;
                                    }
                                }
                            },
                            Err(err) => {
                                warn!("Peer connection closed. Shutting down");
                                return;
                            }
                        }
                    },
                    Some(_) = shutdown_rx.recv() => {
                        let message: MessageBody = Response::Shutdown("Shutdown".into()).into();
                        peer_sender.send(message.into());
                        return;
                    }
                };
            }
        });
    }

    pub fn name(&self) -> &String {
        &self.service_name
    }

    /// Remote method call\
    /// **P** is an argument type. Should be a serializable structure.\
    /// **R** is return type. Should be a deserializable structure
    pub async fn call<P: Serialize, R: DeserializeOwned>(
        &mut self,
        method_name: &str,
        params: &P,
    ) -> Result<R> {
        let message =
            MessageBody::new_call(self.peer_service_name.clone(), method_name.into(), params);

        // Send method call request
        let response = self.peer_sender.call(&message).await?;

        match response.body() {
            // Succesfully performed remote method call
            MessageBody::Response(Response::Return(data)) => {
                match bson::from_bson::<R>(data.clone()) {
                    Ok(data) => Ok(data),
                    Err(err) => {
                        error!("Can't deserialize method response: {}", err.to_string());
                        Err(BusError::InvalidResponse.into())
                    }
                }
            }
            // Got an error from the peer
            MessageBody::Response(Response::Error(err)) => {
                warn!(
                    "Failed to perform a call to `{}::{}`: {}",
                    self.peer_service_name,
                    method_name,
                    err.to_string()
                );
                Err(err.into())
            }
            // Invalid protocol
            r => {
                error!("Invalid Ok response for a method call: {:?}", r);
                Err(BusError::InvalidMessage.into())
            }
        }
    }

    /// Remote signal subscription\
    /// **T** is the signal type. Should be a deserializable structure
    pub async fn subscribe<T>(&mut self, signal_name: &str) -> Result<impl Stream<Item = T>>
    where
        T: DeserializeOwned + Send,
    {
        let message = MessageBody::new_subscription(self.service_name.clone(), signal_name.into());

        let mut subscription_stream = self.peer_sender.subscribe(&message).await?;

        // Match first message
        match subscription_stream
            .next()
            .await
            .context("Subscription stream unexpectedly closed")?
            .body()
        {
            // Succesfully performed remote method call
            MessageBody::Response(Response::Ok) => {
                debug!("Succesfully subscribed to the signal `{}`", signal_name);

                Ok(subscription_stream.map_while(|message| {
                    let bson_body = message.body::<Bson>();

                    if let Ok(body) = bson::from_bson(bson_body.clone()) {
                        Some(body)
                    } else {
                        warn!(
                            "Failed to deserialize subscription body from {:?}",
                            bson_body
                        );
                        None
                    }
                }))
            }
            // Invalid protocol
            r => {
                error!("Invalid Ok response for a signal subscription: {:?}", r);
                Err(BusError::InvalidMessage.into())
            }
        }
    }

    /// Start watching remote state changes\
    /// **T** is the signal type. Should be a deserializable structure\
    /// **Returns** current state value
    pub async fn watch<T>(&mut self, state_name: &str) -> Result<impl Stream<Item = T>>
    where
        T: DeserializeOwned + Send,
    {
        let message = MessageBody::new_watch(self.service_name.clone(), state_name.into());

        let mut subscription_stream = self.peer_sender.subscribe(&message).await?;

        // Match first message
        match subscription_stream
            .next()
            .await
            .context("Watch stream unexpectedly closed")?
            .body()
        {
            // Succesfully performed remote method call
            MessageBody::Response(Response::Ok) => {
                debug!("Succesfully started watching the state `{}`", state_name);

                Ok(subscription_stream.map_while(|message| {
                    let bson_body = message.body::<Bson>();

                    if let Ok(body) = bson::from_bson(bson_body.clone()) {
                        Some(body)
                    } else {
                        warn!("Failed to deserialize watch body from {:?}", bson_body);
                        None
                    }
                }))
            }
            // Invalid protocol
            r => {
                error!("Invalid Ok response for a signal subscription: {:?}", r);
                Err(BusError::InvalidMessage.into())
            }
        }
    }

    /// Set monitor handle
    pub(crate) async fn set_monitor(&mut self, monitor: RpcSender) {
        *self.monitor.lock().await = Some(monitor);
    }

    /// Send message to the Karo monitor if connected
    async fn send_monitor_message(
        monitor: &Arc<Mutex<Option<RpcSender>>>,
        message: &MessageBody,
        direction: MonitorMessageDirection,
        self_name: &String,
        peer_name: &String,
    ) {
        let ref mut monitor = *monitor.lock().await;
        if let Some(monitor) = monitor {
            let (sender, receiver) = match direction {
                MonitorMessageDirection::Outgoing => (self_name.clone(), peer_name.clone()),
                MonitorMessageDirection::Incoming => (peer_name.clone(), self_name.clone()),
            };

            // First we make monitor message, which will be sent as method call parameter...
            let monitor_message = MonitorMessage::new(sender, receiver, message, direction);

            trace!("Sending monitor message: {:?}", message);
            // ..And to call monitor method, we need
            if monitor.call(&message).await.is_err() {
                // Return here if succesfully sent, otherwise reset monitor connection
                return;
            }
        } else {
            return;
        }

        // If reached here, we've failed to send monitor message
        debug!("Monitor disconnected");
        monitor.take();
    }

    /// Close peer connection
    pub async fn close(&mut self) {
        let self_name = self.peer_service_name.clone();
        debug!(
            "Shutting down peer connection to `{}`",
            self.peer_service_name
        );

        self.shutdown_tx.send(()).await;
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        trace!("Peer `{}` connection dropped", self.peer_service_name);

        let shutdown_tx = self.shutdown_tx.clone();

        tokio::spawn(async move {
            let _ = shutdown_tx.send(()).await;
        });
    }
}

impl Debug for Peer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Peer connection to {}", self.peer_service_name)
    }
}
