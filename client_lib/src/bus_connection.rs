use std::collections::HashMap;
use std::error::Error;
use std::os::unix::net::UnixStream as OsStream;
use std::os::unix::prelude::{AsRawFd, FromRawFd};
use std::sync::Arc;

use bson::Bson;
use bytes::BytesMut;
use log::*;
use parking_lot::RwLock;
use passfd::FdPassingExt;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::oneshot::Sender as OneSender;
use tokio::sync::Mutex as AsyncMutex;

use super::method::{Method, MethodCall, MethodTrait};
use super::service_connection::ServiceConnection;
use super::utils;
use common::errors::Error as BusError;
use common::messages::{self, Message, Response, ServiceRequest};
use common::HUB_SOCKET_PATH;

type Shared<T> = Arc<RwLock<T>>;
type CallbackType = Result<Message, Box<dyn Error + Send + Sync>>;

#[derive(Clone)]
pub struct BusConnection {
    service_name: Shared<String>,
    connected_services: Shared<HashMap<String, ServiceConnection>>,
    methods: Shared<HashMap<String, Arc<AsyncMutex<dyn MethodTrait>>>>,
    /// Sender to make calls into the task
    task_tx: Sender<(Message, OneSender<CallbackType>)>,
}

impl BusConnection {
    /// Register service. Tries to register at the hub. Method may fail registering
    /// if the executable is not allowed to register with a given service name, or
    /// Service name is already registered
    pub async fn register(service_name: String) -> Result<Self, Box<dyn Error>> {
        debug!("Registering service `{}`", service_name);

        let mut socket = UnixStream::connect(HUB_SOCKET_PATH).await?;

        let request = messages::make_register_message(service_name.clone());

        let (task_tx, rx) = mpsc::channel(32);
        let mut this = Self {
            service_name: Arc::new(RwLock::new(service_name)),
            connected_services: Arc::new(RwLock::new(HashMap::new())),
            methods: Arc::new(RwLock::new(HashMap::new())),
            task_tx,
        };

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

        this.start(socket, rx);

        Ok(this)
    }

    /// Connect to a service. This call is asynchronous and may fail.
    /// See perfom_service_connection fro actual connection sequence
    pub async fn connect(
        &mut self,
        service_name: String,
    ) -> Result<ServiceConnection, Box<dyn Error + Send + Sync>> {
        debug!("Connecting to a service `{}`", service_name);

        utils::call_task(
            &self.task_tx,
            ServiceRequest::Connect {
                service_name: service_name.clone(),
            }
            .into(),
        )
        .await
        .map(|_| {
            self.connected_services
                .read()
                .get(&service_name)
                .cloned()
                .unwrap()
        })
    }

    /// Register service method
    /// P: Paramters type. Should be Send + Sync + DeserializeOwned
    /// R: Response type. Should be Send + Sync + Serialize
    pub async fn register_method<P, R>(
        &mut self,
        method_name: &String,
    ) -> Result<Receiver<MethodCall<P, R>>, Box<dyn Error>>
    where
        P: DeserializeOwned + Sync + Send + 'static,
        R: Serialize + Sync + Send + 'static,
    {
        let mut methods = self.methods.write();

        if methods.contains_key(method_name) {
            error!(
                "Failed to register method `{}`. Already registered",
                method_name
            );

            return Err(Box::new(BusError::MethodRegistered));
        }

        let (method, tx) = Method::<P, R>::new();

        methods.insert(method_name.clone(), Arc::new(AsyncMutex::new(method)));
        Ok(tx)
    }

    /// Start tokio task to handle incoming requests
    fn start(
        &mut self,
        mut socket: UnixStream,
        mut rx: Receiver<(Message, OneSender<CallbackType>)>,
    ) {
        let mut this = self.clone();

        tokio::spawn(async move {
            let mut bytes = BytesMut::with_capacity(64);

            loop {
                tokio::select! {
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

        match utils::send_receive_message(socket, request).await {
            Ok(Message::Response(Response::Ok)) => {
                info!(
                    "Succesfully connected service `{}` to `{}`",
                    self.service_name.read(),
                    target_service_name
                );
                // Client can receive two types of responses for a connection request:
                // 1. Response::Error if servic eis not allowed to connect
                // 2. Response::Ok and fd right after it if the hub allows the connection
                // We handle second option here

                let fd = socket.as_raw_fd().recv_fd()?;
                let os_stream = unsafe { OsStream::from_raw_fd(fd) };
                let socket = UnixStream::from_std(os_stream).unwrap();

                let new_service_connection = ServiceConnection::new(
                    target_service_name.clone(),
                    socket,
                    self.task_tx.clone(),
                );

                self.connected_services
                    .write()
                    .insert(target_service_name, new_service_connection);

                Ok(Message::Response(Response::Ok))
            }
            Ok(Message::Response(Response::Error(err))) => {
                warn!(
                    "Failed to register service as `{}`: {}",
                    self.service_name.read(),
                    err
                );
                Err(Box::new(err))
            }
            Ok(m) => {
                error!("Invalid response from the hub: {:?}", m);
                Err(Box::new(BusError::InvalidProtocol))
            }
            Err(err) => Err(err),
        }
    }

    // TODO: Rework for Bson to be consistent?
    /// Handle incoming method call
    async fn handle_method_call(&self, method_name: String, params: Bson) -> Message {
        let method = self.methods.read().get(&method_name).cloned();
        if let Some(method) = method {
            match method.lock().await.notify(params).await {
                Ok(response) => Response::Return(response).into(),
                Err(err) => BusError::MethodCallError(err.to_string()).into(),
            }
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

        if let Message::ServiceRequest(request) = message {
            match request {
                // Incoming connection request. Connection socket FD will be coming next
                // So we first read socket, and that create a client handle
                ServiceRequest::Connect { service_name } => {
                    let fd = socket.as_raw_fd().recv_fd().unwrap();
                    let os_stream = unsafe { OsStream::from_raw_fd(fd) };
                    let incoming_socket = UnixStream::from_std(os_stream).unwrap();

                    let new_client = ServiceConnection::new(
                        service_name.clone(),
                        incoming_socket,
                        self.task_tx.clone(),
                    );
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
