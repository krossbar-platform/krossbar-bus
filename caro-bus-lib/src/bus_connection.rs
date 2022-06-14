use std::{
    collections::HashMap,
    error::Error,
    os::unix::{
        net::UnixStream as OsStream,
        prelude::{AsRawFd, FromRawFd},
    },
    sync::Arc,
};

use bson::Bson;
use bytes::BytesMut;
use log::*;
use parking_lot::RwLock;
use passfd::FdPassingExt;
use serde::{de::DeserializeOwned, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
    sync::{
        mpsc::{self, Receiver, Sender},
        oneshot::{self, Sender as OneSender},
    },
};

use super::{
    //method::{Method, MethodCall, MethodTrait},
    peer_connection::PeerConnection,
    utils,
    CallbackType,
};
use caro_bus_common::{
    errors::Error as BusError,
    messages::{self, Message, Response, ServiceRequest},
    HUB_SOCKET_PATH,
};

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
    task_tx: Sender<(Message, OneSender<CallbackType>)>,
}

impl BusConnection {
    /// Register service. Tries to register the service at the hub. The method may fail registering
    /// if the executable is not allowed to register with the given service name, or
    /// service name is already taken
    pub async fn register(service_name: String) -> Result<Self, Box<dyn Error>> {
        debug!("Registering service `{}`", service_name);

        // First connect to a socket
        let mut socket = UnixStream::connect(HUB_SOCKET_PATH).await?;

        let request = messages::make_register_message(service_name.clone());

        let (task_tx, rx) = mpsc::channel(32);
        let mut this = Self {
            service_name: Arc::new(RwLock::new(service_name)),
            connected_services: Arc::new(RwLock::new(HashMap::new())),
            methods: Arc::new(RwLock::new(HashMap::new())),
            task_tx,
        };

        // Send connection request and wait for a response
        match utils::send_receive_message(&mut socket, request).await {
            Ok(Message::Response(Response::Ok)) => {
                info!(
                    "Succesfully registered service as `{}`",
                    this.service_name.read()
                );
            }
            Ok(Message::Response(Response::Error(err))) => {
                warn!(
                    "Failed to register service as `{}`: {}",
                    this.service_name.read(),
                    err
                );
                return Err(Box::new(err));
            }
            Ok(m) => {
                error!("Invalid response from the hub: {:?}", m);
                return Err(Box::new(BusError::InvalidProtocol));
            }
            Err(err) => return Err(err),
        }

        // Start tokio task to handle incoming messages
        this.start_task(socket, rx);

        Ok(this)
    }

    /// Connect to a service. This call is asynchronous and may fail.
    /// See perfom_service_connection for connection sequence
    pub async fn connect(
        &mut self,
        peer_service_name: String,
    ) -> Result<PeerConnection, Box<dyn Error>> {
        debug!("Connecting to a service `{}`", peer_service_name);
        let services = self.connected_services.read();

        // We may have already connected peer. So first we check if connected
        // and if not, perform connection request
        if !services.contains_key(&peer_service_name) {
            let _ = utils::call_task(
                &self.task_tx,
                ServiceRequest::Connect {
                    service_name: peer_service_name.clone(),
                }
                .into(),
            )
            .await
            .map_err(|e| e as Box<dyn Error>)?;
        }

        // We either already had connection, or just created one, so we can
        // return existed connection handle
        Ok(services.get(&peer_service_name).cloned().unwrap())
    }

    pub fn register_method<P, R>(
        &mut self,
        method_name: &String,
        callback: Box<dyn Fn(&P) -> R + Send>,
    ) -> Result<(), Box<dyn Error>>
    where
        P: DeserializeOwned + 'static,
        R: Serialize + 'static,
    {
        let mut rx = self.try_register_method(method_name)?;

        tokio::spawn(async move {
            loop {
                if let Some((params, calback_tx)) = rx.recv().await {
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

            return Err(Box::new(BusError::MethodRegistered));
        }

        let (tx, rx) = mpsc::channel(32);

        methods.insert(method_name.clone(), tx);
        Ok(rx)
    }

    /// Start tokio task to handle incoming requests
    fn start_task(
        &mut self,
        mut socket: UnixStream,
        mut rx: Receiver<(Message, OneSender<CallbackType>)>,
    ) {
        let mut this = self.clone();

        tokio::spawn(async move {
            let mut bytes = BytesMut::with_capacity(64);

            loop {
                tokio::select! {
                    // Read incoming message from the hub
                    read_result = socket.read_buf(&mut bytes) => {
                        if let Err(err) = read_result {
                            error!("Failed to read from a socket: {}. Hub is down", err.to_string());
                            drop(socket);
                            return
                        }

                        if let Some(message) = messages::parse_buffer(&mut bytes) {
                            if let Some(response) = this.handle_bus_message(message, &mut socket).await {
                                socket.write_all(response.bytes().as_slice()).await.unwrap();
                            }
                        }
                    },
                    // Handle service requests and method calls
                    Some((request, callback)) = rx.recv()  => {
                        match request {
                            Message::ServiceRequest(ServiceRequest::Connect { service_name }) => {
                                callback.send(this.perfom_service_connection(service_name, &mut socket).await).unwrap();
                            },
                            Message::MethodCall{ method_name, params } => {
                                callback.send(Ok(this.handle_method_call(method_name, params).await)).unwrap();
                            }
                            m => { error!("Invalid incoming message from a client: {:?}", m) }
                        };
                    }
                };
            }
        });
    }

    /// Perform connection to an another service.
    /// The method may fail if:
    /// 1. The service is not allowed to connect to a target service
    /// 2. Target service is not registered
    async fn perfom_service_connection(
        &self,
        target_service_name: String,
        socket: &mut UnixStream,
    ) -> Result<Message, Box<dyn Error + Sync + Send>> {
        let request = ServiceRequest::Connect {
            service_name: target_service_name.clone(),
        }
        .into();

        // Send connection message and receive response
        match utils::send_receive_message(socket, request).await {
            // Client can receive two types of responses for a connection request:
            // 1. Response::Error if service is not allowed to connect
            // 2. Response::Ok and a socket fd right after the message if the hub allows the connection
            // Handle second case next
            Ok(Message::Response(Response::IncomingClientFd(_))) => {
                info!(
                    "Succesfully connected service `{}` to `{}`",
                    self.service_name.read(),
                    target_service_name
                );

                let fd = socket.as_raw_fd().recv_fd()?;
                let os_stream = unsafe { OsStream::from_raw_fd(fd) };
                let socket = UnixStream::from_std(os_stream).unwrap();

                // Create new service connection handle. Can be used to handle own
                // connection requests by just returning already existing handle
                let new_service_connection =
                    PeerConnection::new(target_service_name.clone(), socket, self.task_tx.clone());

                // Place the handle into the map of existing connections.
                // Caller will use the map to return the handle to the client.
                // See [connect] for the details
                self.connected_services
                    .write()
                    .insert(target_service_name, new_service_connection);

                Ok(Message::Response(Response::Ok))
            }
            // Hub doesn't allow connection
            Ok(Message::Response(Response::Error(err))) => {
                // This is an invalid
                warn!(
                    "Failed to register service as `{}`: {}",
                    self.service_name.read(),
                    err
                );
                Err(Box::new(err))
            }
            // Invalid protocol here
            Ok(m) => {
                error!("Invalid response from the hub: {:?}", m);
                Err(Box::new(BusError::InvalidProtocol))
            }
            // Network error
            // TODO: Close client connection
            Err(err) => Err(err),
        }
    }

    /// Handle incoming method call
    async fn handle_method_call(&self, method_name: String, params: Bson) -> Message {
        let method = self.methods.read().get(&method_name).cloned();

        if let Some(method) = method {
            // Create oneshot channel to receive response
            let (tx, rx) = oneshot::channel();

            // Call user
            method.send((params, tx)).await.unwrap();
            // Await for his respons
            rx.await.unwrap().into()
        } else {
            BusError::MethodRegistered.into()
        }
    }

    /// Handle incoming message from the bus
    async fn handle_bus_message(
        &mut self,
        message: messages::Message,
        socket: &mut UnixStream,
    ) -> Option<Response> {
        trace!("Incoming bus message: {:?}", message);

        if let Message::Response(request) = message {
            match request {
                // Incoming connection request. Connection socket FD will be coming next
                Response::IncomingClientFd(service_name) => {
                    // Read socket handle for p2p connection
                    let fd = socket.as_raw_fd().recv_fd().unwrap();
                    let os_stream = unsafe { OsStream::from_raw_fd(fd) };
                    let incoming_socket = UnixStream::from_std(os_stream).unwrap();

                    // Create a client handle
                    let new_client = PeerConnection::new(
                        service_name.clone(),
                        incoming_socket,
                        self.task_tx.clone(),
                    );

                    // Place the handle into the clients map
                    self.connected_services
                        .write()
                        .insert(service_name, new_client);
                }
                m => error!("Invalid message from the hub: {:?}", m),
            }
        } else {
            error!("Invalid message from the hub: {:?}", message);
        }

        None
    }
}
