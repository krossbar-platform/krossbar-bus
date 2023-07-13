use std::{
    any::type_name,
    collections::HashMap,
    future::Future,
    sync::{Arc, RwLock},
};

use anyhow::Result;
use bson::Bson;
use karo_common_connection::{connection::Connection, one_time_connector::OneTimeConnector};
use log::*;
use serde::{de::DeserializeOwned, Serialize};
use tokio::{
    net::UnixStream,
    sync::{
        broadcast::{self, Sender as BroadcastSender},
        mpsc::{self, Receiver, Sender},
        oneshot::{self, Sender as OneSender},
        watch::{self, Receiver as WatchReceiver},
        RwLock as TokioRwLock,
    },
};

use karo_common_rpc::{
    rpc_connection::RpcConnection, rpc_sender::RpcSender, Message as MessageHandle,
};

use crate::{hub::Hub, monitor::Monitor, peer::Peer, signal::Signal, state::State};

use karo_bus_common::{
    connect::InspectData,
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
    /// Registered methods. Sender is used to send parameters and
    /// receive a result from a callback
    methods: Shared<HashMap<String, Sender<MethodCall>>>,
    /// Registered signals. Sender is used to emit signals to subscribers
    signals: Shared<HashMap<String, BroadcastSender<Message>>>,
    /// Registered states.
    /// Sender is used to nofity state change to subscribers
    /// Receiver used to get current walue when user makes watch request
    states: Shared<HashMap<String, (BroadcastSender<Message>, WatchReceiver<Bson>)>>,
    /// Sender to make calls into the task
    endpoints_tx: Sender<MessageHandle>,
    /// Sender to shutdown bus connection
    shutdown_tx: Sender<()>,
    /// Monitor connection if connected
    monitor: Monitor,
    /// Data for service inspection
    inspect_data: Shared<InspectData>,
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
            methods: Arc::new(RwLock::new(HashMap::new())),
            signals: Arc::new(RwLock::new(HashMap::new())),
            states: Arc::new(RwLock::new(HashMap::new())),
            endpoints_tx: task_tx.clone(),
            shutdown_tx,
            monitor: Monitor::new(service_name),
            inspect_data: Arc::new(RwLock::new(InspectData::new())),
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

    /// Register service method. The function uses BSON internally for requests
    /// and responses.\
    /// **P** is paramtere type. Should be a deserializable structure\
    /// **R** is method return type. Should be a serializable structure
    pub fn register_method<P, R, Ret>(
        &mut self,
        method_name: &str,
        callback: impl Fn(P) -> Ret + Send + Sync + 'static,
    ) -> Result<()>
    where
        P: DeserializeOwned + Send + 'static,
        R: Serialize + Send + 'static,
        Ret: Future<Output = R> + Send,
    {
        let method_name = method_name.into();
        let mut rx = self.update_method_map(&method_name)?;

        // Add the method into the inspection register
        self.inspect_data.write().unwrap().methods.push(format!(
            "{}({}) -> {}",
            method_name,
            type_name::<P>().split("::").last().unwrap_or("Unknown"),
            type_name::<R>().split("::").last().unwrap_or("Unknown")
        ));

        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Some((params, calback_tx)) => {
                        match bson::from_bson::<P>(params) {
                            Ok(params) => {
                                // Receive method call response
                                let result = callback(params).await;

                                // Deserialize and send user response
                                calback_tx
                                    .send(Response::Return(bson::to_bson(&result).unwrap()))
                                    .unwrap();
                            }
                            Err(err) => {
                                warn!(
                                    "Failed to deserialize method call parameters: {}",
                                    err.to_string()
                                );

                                calback_tx
                                    .send(Response::Error(BusError::InvalidParameters(
                                        err.to_string(),
                                    )))
                                    .unwrap();
                            }
                        }
                    }
                    None => {
                        trace!("Method {} shut down", method_name);
                        return;
                    }
                }
            }
        });

        Ok(())
    }

    /// Adds new method to a method map
    fn update_method_map(&mut self, method_name: &String) -> Result<Receiver<MethodCall>> {
        // The function just creates a method handle, which performs type conversions
        // for incoming data and client replies. See [Method] for details
        let mut methods = self.methods.write().unwrap();

        if methods.contains_key(method_name) {
            error!(
                "Failed to register method `{}`. Already registered",
                method_name
            );

            return Err(BusError::AlreadyRegistered.into());
        }

        let (tx, rx) = mpsc::channel(32);

        methods.insert(method_name.clone(), tx);

        info!("Succesfully registered method: {}", method_name);
        Ok(rx)
    }

    /// Register service signal.\
    /// **T** is a signal type. Should be a serializable structure.\
    /// **Returns** [Signal] handle which can be used to emit signal
    pub fn register_signal<T>(&mut self, signal_name: &str) -> Result<Signal<T>>
    where
        T: Serialize + 'static,
    {
        let mut signals = self.signals.write().unwrap();

        if signals.contains_key(signal_name) {
            error!(
                "Failed to register signal `{}`. Already registered",
                signal_name
            );

            return Err(BusError::AlreadyRegistered.into());
        }

        // Add the signal into the inspection register
        self.inspect_data.write().unwrap().signals.push(format!(
            "{}: {}",
            signal_name,
            type_name::<T>().split("::").last().unwrap_or("Unknown"),
        ));

        let (tx, _rx) = broadcast::channel(5);

        signals.insert(signal_name.into(), tx.clone());

        info!("Succesfully registered signal: {}", signal_name);
        Ok(Signal::new(signal_name.into(), tx))
    }

    /// Register service signal.\
    /// **T** is a signal type. Should be a serializable structure.\
    /// **Returns** [State] handle which can be used to change state. Settings the state
    /// will emit state change to watchers.
    pub fn register_state<T>(&mut self, state_name: &str, initial_value: T) -> Result<State<T>>
    where
        T: Serialize + 'static,
    {
        let mut states = self.states.write().unwrap();

        if states.contains_key(state_name) {
            error!(
                "Failed to register state `{}`. Already registered",
                state_name
            );

            return Err(BusError::AlreadyRegistered.into());
        }

        // Add the state into the inspection register
        self.inspect_data.write().unwrap().states.push(format!(
            "{}: {}",
            state_name,
            type_name::<T>().split("::").last().unwrap_or("Unknown"),
        ));

        // Channel to send state update to subscribers
        let (tx, _rx) = broadcast::channel(5);

        // Channel to get current value when someone is subscribing
        let bson = bson::to_bson(&initial_value).unwrap();
        let (watch_tx, watch_rx) = watch::channel(bson);

        states.insert(state_name.into(), (tx.clone(), watch_rx));

        info!("Succesfully registered state: {}", state_name);
        Ok(State::new(state_name.into(), initial_value, tx, watch_tx))
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
        debug!(
            "Service `{}` requested method `{}` call",
            caller_name, method_name
        );

        let seq = handle.id();

        if method_name == karo_bus_common::connect::INSPECT_METHOD {
            handle.reply(&self.handle_inspect_call(seq)).await;
            return;
        }

        let method = self.methods.read().unwrap().get(method_name).cloned();

        let response = if let Some(method) = method {
            // Create oneshot channel to receive response
            let (tx, rx) = oneshot::channel();

            // Call user
            method.send((params.clone(), tx)).await.unwrap();
            // Await for his respons
            rx.await.unwrap().into_message(seq)
        } else {
            BusError::NotRegistered.into_message(seq)
        };

        handle.reply(&response);
    }

    /// Handle incoming method call
    fn handle_inspect_call(&self, seq: u64) -> Message {
        Response::Return(bson::to_bson(&*self.inspect_data.read().unwrap()).unwrap())
            .into_message(seq)
    }

    /// Handle incoming signal subscription
    async fn handle_incoming_signal_subscription(
        &self,
        subscriber_name: &str,
        signal_name: &str,
        handle: &mut MessageHandle,
    ) {
        debug!(
            "Service `{}` requested signal `{}` subscription",
            subscriber_name, signal_name
        );

        let seq = handle.id();
        let signal = self.signals.read().unwrap().get(signal_name).cloned();

        let response = if let Some(signal_sender) = signal {
            // Find subscriber
            match self.peers.read().await.get(subscriber_name) {
                Some(caller) => {
                    caller.start_signal_sending_task(signal_sender.subscribe(), seq);
                    Response::Ok.into_message(seq)
                }
                None => BusError::Internal.into_message(seq),
            }
        } else {
            BusError::NotRegistered.into_message(seq)
        };

        handle.reply(&response);
    }

    /// Handle incoming request to watch state
    async fn handle_incoming_state_watch(
        &self,
        subscriber_name: &str,
        state_name: &str,
        handle: &mut MessageHandle,
    ) {
        debug!(
            "Service `{}` requested state `{}` watch",
            subscriber_name, state_name
        );

        let seq = handle.id();
        let state = self.states.read().unwrap().get(state_name).cloned();

        let response = if let Some((state_change_sender, value_watch)) = state {
            // Find subscriber
            match self.peers.read().await.get(subscriber_name) {
                Some(caller) => {
                    let current_value = value_watch.borrow().clone();

                    caller.start_signal_sending_task(state_change_sender.subscribe(), seq);
                    Response::StateChanged(current_value).into_message(seq)
                }
                None => BusError::Internal.into_message(seq),
            }
        } else {
            BusError::NotRegistered.into_message(seq)
        };

        handle.reply(&response);
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
