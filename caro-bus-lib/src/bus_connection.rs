use std::{
    collections::HashMap,
    error::Error,
    os::unix::{net::UnixStream as OsStream, prelude::FromRawFd},
    sync::Arc,
    time::Duration,
};

use bson::Bson;
use bytes::BytesMut;
use log::*;
use parking_lot::RwLock;
use passfd::tokio::FdPassingExt;
use serde::{de::DeserializeOwned, Serialize};
use tokio::{
    io::AsyncWriteExt,
    net::UnixStream,
    sync::{
        mpsc::{self, Receiver, Sender},
        oneshot::{self, Sender as OneSender},
    },
    time,
};

use super::{
    //method::{Method, MethodCall, MethodTrait},
    peer_connection::PeerConnection,
    utils,
};
use caro_bus_common::{
    call_registry::CallRegistry,
    errors::Error as BusError,
    messages::{self, IntoMessage, Message, MessageBody, Response, ServiceMessage},
    net, HUB_SOCKET_PATH,
};

const RECONNECT_RETRY_PERIOD: Duration = Duration::from_secs(1);

type Shared<T> = Arc<RwLock<T>>;
type MethodCall = (Bson, OneSender<Response>);

/// Bus connection handle. Associated with a service name on the hub.
/// Used to connect to other services
/// and register methods, signals, and states
#[derive(Clone)]
pub struct BusConnection {
    /// Own service name
    service_name: Shared<String>,
    /// Connected services. All these connections are p2p
    connected_services: Shared<HashMap<String, PeerConnection>>,
    /// Registered methods. Sender is used to send parameters and
    /// receive a result from a callback
    methods: Shared<HashMap<String, Sender<MethodCall>>>,
    /// Sender to make calls into the task
    task_tx: Sender<(Message, OneSender<Message>)>,
    /// Registry to make calls and send responses to proper callers
    call_registry: CallRegistry,
}

impl BusConnection {
    /// Register service. Tries to register the service at the hub. The method may fail registering
    /// if the executable is not allowed to register with the given service name, or
    /// service name is already taken
    pub async fn register(service_name: String) -> Result<Self, Box<dyn Error + Send + Sync>> {
        debug!("Registering service `{}`", service_name);

        let (task_tx, rx) = mpsc::channel(32);
        let mut this = Self {
            service_name: Arc::new(RwLock::new(service_name)),
            connected_services: Arc::new(RwLock::new(HashMap::new())),
            methods: Arc::new(RwLock::new(HashMap::new())),
            task_tx,
            call_registry: CallRegistry::new(),
        };

        let mut socket = BusConnection::connect_to_hub().await?;

        this.hub_register(&mut socket).await?;

        // Start tokio task to handle incoming messages
        this.start_task(socket, rx);

        Ok(this)
    }

    /// Connect to a hub socket and send registration request
    /// This method is blocking from the library workflow perspectire: we can't make any hub
    /// calls without previous registration
    async fn hub_register(
        &mut self,
        socket: &mut UnixStream,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        // First connect to a socket
        let self_name = self.service_name.read().clone();
        debug!("Performing service `{}` registration", self_name);

        // Make a message and send to the hub
        let message = Message::new_registration(self_name.clone());
        socket.write_all(message.bytes().as_slice()).await?;

        let mut bytes = BytesMut::with_capacity(64);
        // Wait for hub response
        match net::read_message(socket, &mut bytes).await?.body() {
            MessageBody::Response(Response::Ok) => {
                info!("Succesfully registered service as `{}`", self_name);

                Ok(())
            }
            MessageBody::Response(Response::Error(err)) => {
                warn!("Failed to register service as `{}`: {}", self_name, err);
                return Err(Box::new(BusError::NotAllowed));
            }
            m => {
                error!("Invalid response from the hub: {:?}", m);
                return Err(Box::new(BusError::InvalidProtocol));
            }
        }
    }

    async fn connect_to_hub() -> Result<UnixStream, Box<dyn Error + Send + Sync>> {
        info!("Connecting to a hub socket");

        UnixStream::connect(HUB_SOCKET_PATH)
            .await
            .map_err(|e| e.into())
    }

    async fn reconnect(&mut self) -> Result<UnixStream, Box<dyn Error + Send + Sync>> {
        loop {
            // Try to reconnect to the hub. I fails, we just retry
            match BusConnection::connect_to_hub().await {
                Ok(mut socket) => {
                    info!("Succesfully reconnected to the hub");

                    // Try to connect and register
                    return self.hub_register(&mut socket).await.and(Ok(socket));
                }
                Err(_) => {
                    debug!("Failed to reconnect to a hub socket. Will try again");

                    time::sleep(RECONNECT_RETRY_PERIOD).await;
                    continue;
                }
            }
        }
    }

    /// Start tokio task to handle incoming requests
    fn start_task(
        &mut self,
        mut socket: UnixStream,
        mut task_rx: Receiver<(Message, OneSender<Message>)>,
    ) {
        let mut this = self.clone();

        tokio::spawn(async move {
            let mut bytes = BytesMut::with_capacity(64);

            loop {
                tokio::select! {
                    // Read incoming message from the hub
                    read_result = net::read_message(&mut socket, &mut bytes) => {
                        match read_result {
                            Ok(message) => {
                                trace!("Got a message from the hub: {:?}", message);
                                let response_seq = message.seq();

                                match message.body() {
                                    MessageBody::ServiceMessage(ServiceMessage::IncomingPeerFd { peer_service_name }) =>
                                    {
                                        trace!("Incoming file descriptor for a peer: {}", peer_service_name);
                                        this.receive_and_register_peer_fd(peer_service_name.clone(), &mut socket)
                                        .await.unwrap();

                                        if this.call_registry.has_call(message.seq()) {
                                            this.call_registry.resolve(message);
                                        }
                                    }
                                    // If got a response to a call, handle it by call_registry. Otherwise it's
                                    // an incoming call. Use [handle_bus_message]
                                    MessageBody::Response(_) =>
                                    {
                                        this.call_registry.resolve(message)
                                    },
                                    _ => if let Some(response) = this.handle_bus_message(message, &mut socket).await {
                                        let message = response.into_message(response_seq);

                                        if let Err(err) = socket.write_all(message.bytes().as_slice()).await {
                                            warn!("Hub closed its socket: {}. Trying to reconnect", err.to_string());

                                            match this.reconnect().await {
                                                Ok(new_socket) => socket = new_socket,
                                                Err(err) => {
                                                    error!("Failed to reconnect to a hub: {}", err.to_string());
                                                    return
                                                }
                                            }
                                        }
                                    }
                                }
                            },
                            Err(err) => {
                                warn!("Hub closed its socket: {}. Trying to reconnect", err.to_string());

                                match this.reconnect().await {
                                    Ok(new_socket) => socket = new_socket,
                                    Err(err) => {
                                        error!("Failed to reconnect to a hub: {}", err.to_string());
                                        return
                                    }
                                }
                            }
                        }
                    },
                    Some((request, callback_tx)) = task_rx.recv() => {
                        trace!("Service task message: {:?}", request);

                        match request.body() {
                            MessageBody::ServiceMessage(ServiceMessage::Connect { .. }) => {
                                if let Err(_) = this.call_registry.call(&mut socket, request, callback_tx).await {
                                    info!("Service connection received shutdown request");
                                    drop(socket);
                                    return
                                }
                            },
                            // Someone wants us to shutdown
                            MessageBody::Response(Response::Shutdown) => {
                                info!("Service connection received shutdown request");
                                callback_tx.send(Response::Ok.into_message(999)).unwrap();

                                drop(socket);
                                return
                            }
                            m => { error!("Invalid client message: {:?}", m) }
                        };
                    },
                };
            }
        });
    }

    /// Perform connection to an another service.
    /// The method may fail if:
    /// 1. The service is not allowed to connect to a target service
    /// 2. Target service is not registered
    pub async fn connect(
        &mut self,
        peer_service_name: String,
    ) -> Result<PeerConnection, Box<dyn Error>> {
        debug!("Connecting to a service `{}`", peer_service_name);

        // We may have already connected peer. So first we check if connected
        // and if not, perform connection request
        if !self
            .connected_services
            .read()
            .contains_key(&peer_service_name)
        {
            match utils::call_task(
                &self.task_tx,
                Message::new_connection(peer_service_name.clone()),
            )
            .await
            .map_err(|e| e as Box<dyn Error>)?
            .body()
            {
                // Client can receive two types of responses for a connection request:
                // 1. Response::Error if service is not allowed to connect
                // 2. Response::Ok and a socket fd right after the message if the hub allows the connection
                // Handle second case next
                MessageBody::ServiceMessage(ServiceMessage::IncomingPeerFd { .. }) => {
                    info!("Received connection confirmation to `{}` from the hub. Tring to receive peer socket",
                        peer_service_name);
                }
                // Hub doesn't allow connection
                MessageBody::Response(Response::Error(err)) => {
                    // This is an invalid
                    warn!(
                        "Failed to register service as `{}`: {}",
                        self.service_name.read(),
                        err
                    );
                    return Err(Box::new(BusError::NotAllowed));
                }
                // Invalid protocol here
                m => {
                    error!("Invalid response from the hub: {:?}", m);
                    return Err(Box::new(BusError::InvalidProtocol));
                }
            }
        }

        // We either already had connection, or just created one, so we can
        // return existed connection handle
        Ok(self
            .connected_services
            .read()
            .get(&peer_service_name)
            .cloned()
            .unwrap())
    }

    pub fn register_method<P, R>(
        &mut self,
        method_name: &String,
        callback: impl Fn(&P) -> R + Send + 'static,
    ) -> Result<(), Box<dyn Error>>
    where
        P: DeserializeOwned + 'static,
        R: Serialize + 'static,
    {
        let method_name = method_name.clone();
        let mut rx = self.try_register_method(&method_name)?;

        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Some((params, calback_tx)) => {
                        match bson::from_bson::<P>(params) {
                            Ok(params) => {
                                // Receive method call response
                                let result = callback(&params);

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
                                    .send(Response::Error(BusError::InvalidParameters))
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

    /// Register service method. Returns a Future, which cna be waited for method calls
    /// *Returns* receiver, which can be used to pull for method calls
    fn try_register_method(
        &mut self,
        method_name: &String,
    ) -> Result<Receiver<MethodCall>, Box<dyn Error>> {
        // The function just creates a method handle, which performs type conversions
        // for incoming data and client replies. See [Method] for details
        let mut methods = self.methods.write();

        if methods.contains_key(method_name) {
            error!(
                "Failed to register method `{}`. Already registered",
                method_name
            );

            return Err(Box::new(BusError::MethodNotRegistered));
        }

        let (tx, rx) = mpsc::channel(32);

        methods.insert(method_name.clone(), tx);
        Ok(rx)
    }

    /// Handle messages incoming form an existent peer connection
    pub async fn handle_peer_message(
        &self,
        message: messages::Message,
    ) -> Result<Message, Box<dyn Error + Send + Sync>> {
        match message.body() {
            // Method call
            MessageBody::MethodCall {
                method_name,
                params,
            } => Ok(self
                .handle_method_call(method_name, params, message.seq())
                .await),
            m => {
                error!("Invalid incoming message from library client: {:?}", m);
                Err(BusError::InvalidProtocol.into())
            }
        }
    }

    pub fn remove_peer(&mut self, peer_name: String) {
        info!("Service client `{}` disconnected", peer_name);

        self.connected_services.write().remove(&peer_name);
    }

    /// Handle incoming method call
    async fn handle_method_call(&self, method_name: &String, params: &Bson, seq: u64) -> Message {
        let method = self.methods.read().get(method_name).cloned();

        if let Some(method) = method {
            // Create oneshot channel to receive response
            let (tx, rx) = oneshot::channel();

            // Call user
            method.send((params.clone(), tx)).await.unwrap();
            // Await for his respons
            rx.await.unwrap().into_message(seq)
        } else {
            BusError::MethodNotRegistered.into_message(seq)
        }
    }

    /// Handle incoming message from the bus
    async fn handle_bus_message(
        &mut self,
        message: messages::Message,
        socket: &mut UnixStream,
    ) -> Option<Response> {
        trace!("Incoming bus message: {:?}", message);

        match message.body() {
            // Incoming connection request. Connection socket FD will be coming next
            MessageBody::ServiceMessage(ServiceMessage::IncomingPeerFd { peer_service_name }) => {
                trace!(
                    "Received peer connection message from the hub. Trying to receive peer socket"
                );

                if let Err(err) = self
                    .receive_and_register_peer_fd(peer_service_name.clone(), socket)
                    .await
                {
                    error!(
                        "Failed to receive peer file descriptor: {}",
                        err.to_string()
                    );
                }
            }
            MessageBody::Response(Response::Error(err)) => {
                error!("Hub send us an error: {:?}", err)
            }
            m => error!("Invalid message from the hub: {:?}", m),
        };

        None
    }

    /// Well, receive client socket descriptor and create an entry in clients map
    async fn receive_and_register_peer_fd(
        &self,
        peer_service_name: String,
        socket: &mut UnixStream,
    ) -> Result<Message, Box<dyn Error + Sync + Send>> {
        match socket.recv_fd().await {
            Ok(peer_fd) => {
                trace!("Succesfully received peer socket from the hub");

                let os_stream = unsafe { OsStream::from_raw_fd(peer_fd) };
                // Should not forget about making non-blocking.
                // This cost me two days of life
                os_stream.set_nonblocking(true)?;

                let incoming_socket = UnixStream::from_std(os_stream).unwrap();

                // Create new service connection handle. Can be used to handle own
                // connection requests by just returning already existing handle
                let new_service_connection =
                    PeerConnection::new(peer_service_name.clone(), incoming_socket, self.clone());

                // Place the handle into the map of existing connections.
                // Caller will use the map to return the handle to the client.
                // See [connect] for the details
                self.connected_services
                    .try_write()
                    .unwrap()
                    .insert(peer_service_name, new_service_connection);

                Ok(Response::Ok.into_message(999))
            }
            Err(err) => {
                error!("Failed to receive peer socket: {:?}", err);
                Err(Box::new(err))
            }
        }
    }

    pub async fn close(&mut self) {
        debug!(
            "Shutting down service connection for `{:?}`",
            self.service_name.read()
        );

        for peer in self.connected_services.write().values_mut() {
            peer.close().await
        }

        utils::call_task(&self.task_tx, Response::Shutdown.into_message(999))
            .await
            .unwrap();
    }
}
