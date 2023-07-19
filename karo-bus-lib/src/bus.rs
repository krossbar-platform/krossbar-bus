use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use anyhow::Result;
use bson::Bson;
use karo_common_connection::{connection::Connection, one_time_connector::OneTimeConnector};
use log::*;
use tokio::{
    net::UnixStream,
    sync::{
        mpsc::{self, Receiver, Sender},
        oneshot::Sender as OneSender,
        RwLock as TokioRwLock,
    },
};

use karo_common_rpc::{
    rpc_connection::RpcConnection, rpc_sender::RpcSender, Message as MessageHandle,
};

use crate::{
    connections::{hub::Hub, peer::Peer},
    endpoints::Endpoints,
    monitor::Monitor,
};

use karo_bus_common::{
    errors::Error as BusError,
    messages::{IntoMessage, Message, MessageBody, Response, ServiceMessage},
    monitor::MONITOR_SERVICE_NAME,
};

type Shared<T> = Arc<RwLock<T>>;
type MethodCall = (Bson, OneSender<Response>);

/// Bus connection handle. Associated with a service name at the hub.
/// Use to connect to other services,
/// register methods, signals, and states
#[derive(Clone)]
pub struct Bus {
    /// Own service name
    service_name: String,
    /// Connected services. All these connections are p2p
    peers: Arc<TokioRwLock<HashMap<String, Peer>>>,
    /// User service endpoints
    endpoints: Endpoints,
    /// Sender to make calls into the task
    endpoints_tx: Sender<MessageHandle>,
    /// Sender to shutdown bus connection
    shutdown_tx: Sender<()>,
    /// Monitor connection if connected
    monitor: Monitor,
    /// Hub writer to perform outgoing connections
    hub_sender: RpcSender,
}

impl Bus {
    pub fn service_name(&self) -> &String {
        &self.service_name
    }

    /// Register service. Tries to register the service at the hub. The method may fail registering
    /// if the executable is not allowed to register with the given service name, or
    /// service name is already taken
    pub async fn register(service_name: &str) -> Result<Self> {
        debug!("Registering service `{}`", service_name);

        let (task_tx, rx) = mpsc::channel(32);
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

        let hub_connection = Hub::new(service_name).await?;
        let mut this = Self {
            service_name: service_name.into(),
            peers: Arc::new(TokioRwLock::new(HashMap::new())),
            endpoints: Endpoints::new(),
            endpoints_tx: task_tx.clone(),
            shutdown_tx,
            monitor: Monitor::new(service_name),
            hub_sender: hub_connection.sender(),
        };

        // Start tokio task to handle incoming messages
        this.start(hub_connection, rx, shutdown_rx);

        Ok(this)
    }

    /// Start tokio task to handle incoming requests
    fn start(
        &mut self,
        mut hub_connection: RpcConnection,
        mut interfaces_rx: Receiver<MessageHandle>,
        mut shutdown_rx: Receiver<()>,
    ) {
        let mut this = self.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    // Read incoming message from the hub
                    Ok(message) = hub_connection.read() => {
                        trace!("Got a message from the hub: {:?}", message);

                        this.handle_bus_message(message, &mut hub_connection).await;
                    },
                    Some(message_handle) = interfaces_rx.recv() => {
                        trace!("Service task message: {:?}", message_handle.body::<Bson>());

                        this.handle_task_message(message_handle).await;
                    },
                    Some(_) = shutdown_rx.recv() => {
                        drop(hub_connection);
                        return
                    }
                };
            }
        });
    }

    /// Perform connection to an another service.
    /// The method may fail if:
    /// 1. The service is not allowed to connect to a target service
    /// 2. Target service is not registered or doesn't exist
    pub async fn connect(&mut self, peer_service_name: &str) -> Result<Peer> {
        self.connect_perform(peer_service_name, false).await
    }

    /// Perform connection to an another service. Wait for service to connect.
    /// The method may fail if:
    /// 1. The service is not allowed to connect to a target service
    /// 2. Target service doesn't exist
    pub async fn connect_await(&mut self, peer_service_name: &str) -> Result<Peer> {
        self.connect_perform(peer_service_name, true).await
    }

    /// Perform all communication for connection request
    async fn connect_perform(
        &mut self,
        peer_service_name: &str,
        await_connection: bool,
    ) -> Result<Peer> {
        debug!("Connecting to a service `{}`", peer_service_name);

        // We may have already connected peer. So first we check if connected
        // and if not, perform connection request
        if !self.peers.read().await.contains_key(peer_service_name) {
            // Call hub and get connection response
            let mut connection_response = self
                .hub_sender
                .call(&Message::new_connection(
                    peer_service_name.into(),
                    await_connection,
                ))
                .await?;

            match connection_response.body() {
                // Client can receive two types of responses for a connection request:
                // 1. Response::Error if service is not allowed to connect
                // 2. Response::Ok and a socket fd right after the message if the hub allows the connection
                // Handle second case next
                MessageBody::ServiceMessage(ServiceMessage::PeerFd(fd)) => {
                    info!("Connection to `{}` succeded", peer_service_name);

                    let stream = connection_response.take_fd();

                    if stream.is_none() {
                        error!("No file descriptor in a peer FD message");
                        return Err(BusError::Internal.into());
                    }

                    self.register_peer_fd(peer_service_name, stream.unwrap(), true)
                        .await;
                }
                // Hub doesn't allow connection
                MessageBody::Response(Response::Error(err)) => {
                    // This is an invalid
                    warn!("Failed to connect to `{}`: {}", &peer_service_name, err);
                    return Err(err.into());
                }
                // Invalid protocol here
                m => {
                    error!("Invalid response from the hub: {:?}", m);
                    return Err(BusError::InvalidMessage.into());
                }
            }
        }

        // We either already had connection, or just created one, so we can
        // return existed connection handle
        Ok(self
            .peers
            .read()
            .await
            .get(peer_service_name)
            .cloned()
            .unwrap())
    }

    /// Handle messages incoming form an existent peer connection
    async fn handle_task_message(&mut self, mut message_handle: MessageHandle) {
        match message_handle.body() {
            MessageBody::MethodCall {
                caller_name,
                method_name,
                params,
            } => {
                self.handle_method_call(&caller_name, &method_name, &params, &mut message_handle)
                    .await;
            }
            MessageBody::SignalSubscription {
                subscriber_name,
                signal_name,
            } => {
                let response = self
                    .handle_incoming_signal_subscription(
                        &subscriber_name,
                        &signal_name,
                        &mut message_handle,
                    )
                    .await;
            }
            MessageBody::StateSubscription {
                subscriber_name,
                state_name,
            } => {
                let response = self
                    .handle_incoming_state_watch(&subscriber_name, &state_name, &mut message_handle)
                    .await;
            }
            // Peer connection wants us to shut it down
            MessageBody::Response(Response::Shutdown(peer_name)) => {
                info!(
                    "Service connection received shutdown request from {}",
                    peer_name
                );
                self.remove_peer(peer_name.clone()).await;
            }
            m => {
                error!("Invalid client message: {:?}", m)
            }
        };
    }

    /// Handle incoming method call
    async fn handle_method_call(
        &self,
        caller_name: &str,
        method_name: &str,
        params: &Bson,
        handle: &mut MessageHandle,
    ) {
        self.endpoints
            .handle_method_call(caller_name, method_name, params, handle);
    }

    /// Handle incoming method call
    fn handle_inspect_call(&self, seq: u64) -> Message {
        self.endpoints.handle_inspect_call(seq)
    }

    /// Handle incoming signal subscription
    async fn handle_incoming_signal_subscription(
        &self,
        subscriber_name: &str,
        signal_name: &str,
        handle: &mut MessageHandle,
    ) {
        let seq = handle.id();

        match self.peers.read().await.get(subscriber_name) {
            Some(caller) => {
                self.endpoints.handle_incoming_signal_subscription(
                    subscriber_name,
                    signal_name,
                    handle,
                    caller,
                );
            }
            None => {
                handle.reply(&BusError::Internal.into_message(seq));
            }
        }
    }

    /// Handle incoming request to watch state
    async fn handle_incoming_state_watch(
        &self,
        subscriber_name: &str,
        state_name: &str,
        handle: &mut MessageHandle,
    ) {
        let seq = handle.id();

        match self.peers.read().await.get(subscriber_name) {
            Some(caller) => {
                self.endpoints.handle_incoming_state_watch(
                    subscriber_name,
                    state_name,
                    handle,
                    caller,
                );
            }
            None => {
                handle.reply(&BusError::Internal.into_message(seq));
            }
        }
    }

    async fn remove_peer(&mut self, peer_name: String) {
        info!("Service client `{}` disconnected", peer_name);

        self.peers.write().await.remove(&peer_name);
    }

    /// Handle incoming message from the bus
    async fn handle_bus_message(
        &mut self,
        mut message_handle: MessageHandle,
        hub_connection: &mut RpcConnection,
    ) {
        trace!("Incoming bus message: {:?}", message_handle);

        match message_handle.body() {
            // Incoming connection request. Connection socket FD will be coming next
            MessageBody::ServiceMessage(ServiceMessage::IncomingPeerFd { peer_service_name }) => {
                trace!("Incoming file descriptor for a peer: {}", peer_service_name);

                let stream = message_handle.take_fd();
                if stream.is_none() {
                    error!("Failed to retrieve FD from a peer FD message");
                    return;
                }

                // Monitor connection has its own flow
                if peer_service_name == MONITOR_SERVICE_NAME {
                    self.register_monitor(stream.unwrap(), hub_connection).await;
                    return;
                }

                self.register_peer_fd(&peer_service_name, stream.unwrap(), false)
                    .await;
            }
            // If got a response to a call, handle it by call_registry. Otherwise it's
            // an incoming call. Use [handle_bus_message]
            m => error!("Invalid message from the hub: {:?}", m),
        };
    }

    /// Register new [Peer] with a given unix socket file descriptor
    async fn register_peer_fd(&self, peer_service_name: &str, stream: UnixStream, outgoing: bool) {
        // Create new service connection handle. Can be used to handle own
        // connection requests by just returning already existing handle
        let mut new_service_connection = Peer::new(
            self.service_name.clone(),
            peer_service_name.into(),
            Some(stream),
            self.endpoints_tx.clone(),
            self.hub_sender.clone(),
        )
        .await
        .unwrap();

        new_service_connection.set_monitor(&self.monitor).await;

        // Place the handle into the map of existing connections.
        // Caller will use the map to return the handle to the client.
        // See [connect] for the details
        self.peers
            .try_write()
            .unwrap()
            .insert(peer_service_name.into(), new_service_connection);
    }

    /// Register incoming Karo monitor connections
    async fn register_monitor(&mut self, stream: UnixStream, hub_connection: &mut RpcConnection) {
        debug!("Incoming monitor connection");

        // This can never fail because we already have a stream
        let monitor_connection = Connection::new(Box::new(OneTimeConnector::new(stream)))
            .await
            .unwrap();

        // Set active connection to the common monitor handle
        self.monitor.set_connection(monitor_connection);

        // Set monitor handle to the hub
        hub_connection.set_monitor(Box::new(self.monitor.clone_for_service("karo.bus")));

        // Set monitor handle to the peers
        for peer in self.peers.write().await.values_mut() {
            peer.set_monitor(&self.monitor).await;
        }
    }

    /// Close service connection
    pub async fn close(&mut self) {
        debug!(
            "Shutting down service connection for `{}`",
            self.service_name
        );

        let _ = self.shutdown_tx.send(()).await;
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let shutdown_tx = self.shutdown_tx.clone();

        tokio::spawn(async move {
            let _ = shutdown_tx.send(()).await;
        });
    }
}
