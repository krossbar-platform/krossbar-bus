use std::{collections::HashMap, io::ErrorKind, pin::Pin, time::Duration};

use futures::{future, select, stream::FuturesUnordered, Future, FutureExt, StreamExt};
use krossbar_bus_common::{get_hub_socket_path, message::HubMessage, HUB_REGISTER_METHOD};
use log::{debug, error, info, warn};
use serde::{de::DeserializeOwned, Serialize};
use tokio::{net::UnixStream, signal::ctrl_c, time};

use krossbar_common_rpc::{
    request::{Body, RpcRequest},
    rpc::Rpc,
};

use crate::{
    endpoints::{signal::Signal, state::State, Endpoints},
    signal::AsyncSignal,
};

use super::{
    client::{ClientEvent, ClientHandle},
    Client,
};

const RECONNECT_ATTEMP_COOLDOWN_MS: u64 = 1000;

type ClientsStreamType =
    FuturesUnordered<Pin<Box<dyn Future<Output = (ClientEvent, ClientHandle)> + Send>>>;

pub struct Service {
    service_name: String,
    endpoints: Endpoints,
    client_map: HashMap<String, Client>,
    client_poll_handles: ClientsStreamType,
    hub_connection: Rpc,
    reconnect_signal: AsyncSignal<crate::Result<()>>,
}

impl Service {
    pub async fn new(service_name: &str) -> crate::Result<Self> {
        let hub_connection = Self::hub_connect(service_name).await?;

        let clients: ClientsStreamType = FuturesUnordered::new();
        clients.push(Box::pin(future::pending()));

        Ok(Self {
            service_name: service_name.to_owned(),
            endpoints: Endpoints::new(),
            client_map: HashMap::new(),
            client_poll_handles: clients,
            hub_connection,
            reconnect_signal: AsyncSignal::new(),
        })
    }

    pub async fn connect(&mut self, service_name: &str) -> crate::Result<Client> {
        self.connect_impl(service_name, false).await
    }

    pub async fn connect_await(&mut self, service_name: &str) -> crate::Result<Client> {
        self.connect_impl(service_name, true).await
    }

    pub async fn connect_impl(&mut self, service_name: &str, wait: bool) -> crate::Result<Client> {
        if let Some(handle) = self.client_map.get_mut(service_name) {
            handle.make_outgoing();
            return Ok(handle.clone());
        }

        let connection_request = ClientHandle::connect(
            service_name,
            self.hub_connection.writer().clone(),
            self.reconnect_signal.clone(),
            wait,
        );

        // Need to pull self RPC connection to receive hub response
        let handle = select! {
            handle = connection_request
            .fuse() => match handle {
                Ok(handle) => handle,
                Err(e) => return Err(e)
            },
            _ = self.poll().fuse() => { panic!("Unexpected self poll return");}
        };

        // One handle for a client
        let result = handle.client_handle();

        info!("Succesfully connected to a client: {service_name}");
        // Another handle to pull
        self.schedule_client_poll(handle);

        // And last handle to track clients and maybe spawn new clients
        self.client_map
            .insert(service_name.to_owned(), result.clone());

        Ok(result)
    }

    pub fn register_method<P, R, Fr, F>(&mut self, name: &str, func: F) -> crate::Result<()>
    where
        P: DeserializeOwned + 'static + Send,
        R: Serialize,
        Fr: Future<Output = R> + Send,
        F: FnMut(P) -> Fr + 'static + Send,
    {
        self.endpoints.register_method(name, func)
    }

    pub fn register_signal<T: Serialize>(&mut self, name: &str) -> crate::Result<Signal<T>> {
        self.endpoints.register_signal(name)
    }

    pub fn register_state<T: Serialize>(
        &mut self,
        name: &str,
        value: T,
    ) -> crate::Result<State<T>> {
        self.endpoints.register_state(name, value)
    }

    fn schedule_client_poll(&mut self, mut poll_handle: ClientHandle) {
        self.client_poll_handles.push(Box::pin(async move {
            let event = poll_handle.poll().await;
            (event, poll_handle)
        }));
    }

    pub async fn run(mut self) -> crate::Result<()> {
        loop {
            self.poll().await?
        }
    }

    pub async fn poll(&mut self) -> crate::Result<()> {
        select! {
            client_request = self.client_poll_handles.next() => {
                let (event, poll_handle) = client_request.unwrap();
                match event {
                    // Incomming clietn message
                    ClientEvent::Message(service_name, request) => {
                        debug!("Service {} got incoming message from {}: {:?}",
                               self.service_name, service_name, request.message_id());

                        self.handle_incoming_call(&service_name, request).await;
                        // Reschedule client
                        self.schedule_client_poll(poll_handle);
                    },
                    // Client disconnected
                    ClientEvent::Disconnect(service_name) => {
                        info!("Client {service_name} disconnected");
                        self.client_map.remove(&service_name);
                    }
                }
            }
            hub_request = self.hub_connection.poll().fuse() => {
                match hub_request {
                    // Incoming peer connection
                    Some(request) => {
                        self.handle_new_connection(request)
                    },
                    // Hub disconnected
                    None => {
                        match Self::hub_connect(&self.service_name).await {
                            Ok(hub_connection) => {
                                self.hub_connection.on_reconnected(hub_connection).await;
                                self.reconnect_signal.emit(Ok(())).await
                            },
                            Err(e) => self.reconnect_signal.emit(Err(e)).await
                        }
                    }
                }
            },
            _ = ctrl_c().fuse() => {
                return Ok(())
            }
        }

        Ok(())
    }

    fn handle_new_connection(&mut self, mut request: RpcRequest) {
        let (service_name, stream) = match request.take_body().unwrap() {
            Body::Fd(service_name, stream) => (service_name, stream),
            _ => {
                error!("Invalid hub request: not a connection request. Please report a bug",);
                return;
            }
        };

        let poll_handle = ClientHandle::new(
            service_name.clone(),
            Rpc::new(stream),
            self.hub_connection.writer().clone(),
            self.reconnect_signal.clone(),
            false,
        );

        if self.client_map.contains_key(&service_name) {
            warn!("Multiple service {} connection requested", service_name)
        } else {
            info!("Added new incoming connection from: {service_name}");

            self.client_map
                .insert(service_name, poll_handle.client_handle());

            self.schedule_client_poll(poll_handle);
        }
    }

    async fn handle_incoming_call(&mut self, _service_name: &String, request: RpcRequest) {
        self.endpoints.handle_call(request).await
    }

    async fn hub_connect(service_name: &str) -> crate::Result<Rpc> {
        let hub_socket_path = get_hub_socket_path();
        loop {
            info!("Connecting to hub at: {}", hub_socket_path);

            match UnixStream::connect(&hub_socket_path).await {
                Ok(stream) => {
                    let mut rpc = Rpc::new(stream);

                    let registration_response = rpc
                        .call::<HubMessage, ()>(
                            HUB_REGISTER_METHOD,
                            &HubMessage::Register {
                                service_name: service_name.to_owned(),
                            },
                        )
                        .await?;

                    let mut fused_response = registration_response.fuse();
                    select! {
                        // Immediate hub response
                        response = fused_response => {
                            return match response {
                                Ok(()) => {
                                    info!("Succesfully registered as a service: {service_name}");
                                    Ok(rpc)
                                },
                                Err(e) => {
                                    return Err(e)
                                }
                            }
                        }
                        // Need to run `poll` here to receive response
                        hub_return = rpc.poll().fuse() => {
                            if hub_return.is_none() {
                                warn!("Hub dropped the socket during connection request. Trying to receive the response");

                                // Hub dropped the socket, but we still didn't receive the data, so we
                                // want to try it again. Some code duplication
                                select! {
                                    response = fused_response => {
                                        return match response {
                                            Ok(()) => {
                                                info!("Succesfully registered as a service: {service_name}");
                                                Ok(rpc)
                                            },
                                            Err(e) => {
                                                return Err(e)
                                            }
                                        }
                                    },
                                    _ = time::sleep(Duration::from_millis(RECONNECT_ATTEMP_COOLDOWN_MS)).fuse() => {
                                        error!("Hub connection timeout");

                                        return Err(crate::Error::ProtocolError);
                                    }
                                }
                            }
                            return Err(crate::Error::InternalError("Hub connection early return".to_owned()));
                        },
                    }
                }
                Err(e)
                    if e.kind() == ErrorKind::NotFound
                        || e.kind() == ErrorKind::ConnectionRefused =>
                {
                    warn!(
                        "Failed to connect to the hub. Hub is down: {}. Reconnect in {} ms",
                        e.to_string(),
                        RECONNECT_ATTEMP_COOLDOWN_MS
                    );
                    time::sleep(Duration::from_millis(RECONNECT_ATTEMP_COOLDOWN_MS)).await;
                    continue;
                }
                Err(e) => {
                    error!(
                        "Failed to connect to the hub. Critical error: {}",
                        e.to_string()
                    );
                    return Err(crate::Error::NotAllowed);
                }
            };
        }
    }
}