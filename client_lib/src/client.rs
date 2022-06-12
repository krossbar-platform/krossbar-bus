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

use super::connection::Connection;
use super::method::{Method, MethodCall, MethodTrait};
use super::utils;
use common::errors::Error as BusError;
use common::messages::{self, Message, Response};
use common::HUB_SOCKET_PATH;

type Shared<T> = Arc<RwLock<T>>;

#[derive(Debug, Clone)]
enum ClientRequest {
    Connect(
        String,
        Sender<Result<Connection, Box<dyn Error + Send + Sync>>>,
    ),
    MethodCall(String, Bson),
    SignalSubscription(String),
}

#[derive(Clone)]
pub struct BusClient {
    service_name: Shared<String>,
    methods: Shared<HashMap<String, Shared<dyn MethodTrait>>>,
    self_tx: Sender<ClientRequest>,
}

impl BusClient {
    pub async fn register(service_name: String) -> Result<Self, Box<dyn Error>> {
        debug!("Registering service `{}`", service_name);

        let mut socket = UnixStream::connect(HUB_SOCKET_PATH).await?;

        let request = messages::make_register_message(service_name.clone());

        let (self_tx, rx) = mpsc::channel(32);
        let mut this = Self {
            service_name: Arc::new(RwLock::new(service_name)),
            methods: Arc::new(RwLock::new(HashMap::new())),
            self_tx,
        };

        match utils::send_receive(&mut socket, request).await {
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

    pub async fn connect(
        &mut self,
        service_name: String,
    ) -> Result<Connection, Box<dyn Error + Send + Sync>> {
        debug!("Connection to a service `{}`", service_name);

        let (tx, mut rx) = mpsc::channel(32);
        self.self_tx
            .send(ClientRequest::Connect(service_name, tx))
            .await?;

        rx.recv().await.unwrap()
    }

    pub async fn register_method<ParamsType, ResponseType>(
        &mut self,
        method_name: &String,
    ) -> Result<Receiver<MethodCall<ParamsType, ResponseType>>, Box<dyn Error>>
    where
        ParamsType: DeserializeOwned + Sync + Send + 'static,
        ResponseType: Serialize + Sync + Send + 'static,
    {
        let mut methods = self.methods.write();

        if methods.contains_key(method_name) {
            error!(
                "Failed to register method `{}`. Already registered",
                method_name
            );

            return Err(Box::new(BusError::MethodRegistered));
        }

        let (method, tx) = Method::<ParamsType, ResponseType>::new();

        methods.insert(method_name.clone(), Arc::new(RwLock::new(method)));
        Ok(tx)
    }

    fn start(&mut self, mut socket: UnixStream, mut rx: Receiver<ClientRequest>) {
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
                            if let Some(response) = this.handle_client_message(message).await {
                                socket.write_all(response.bytes().as_slice()).await.unwrap();
                            }
                        }
                    },
                    request = rx.recv()  => {
                        match request.unwrap() {
                            ClientRequest::Connect(service_name, tx) => {
                                tx.send(this.handle_connect(service_name, &mut socket).await).await.unwrap();
                            }
                            ClientRequest::MethodCall(method_name, params) => {
                                let response = this.handle_method_call(method_name, params).await;
                                socket.write_all(response.as_slice()).await.unwrap();
                            },
                            ClientRequest::SignalSubscription(_signal_name) => {}
                        };
                    }
                };
            }
        });
    }

    async fn handle_connect(
        &self,
        service_name: String,
        socket: &mut UnixStream,
    ) -> Result<Connection, Box<dyn Error + Sync + Send>> {
        let request = messages::make_connection_message(service_name.clone());

        match utils::send_receive(socket, request).await {
            Ok(Message::Response(Response::Ok)) => {
                info!(
                    "Succesfully connected service `{}` to `{}`",
                    self.service_name.read(),
                    service_name
                );
                // Client can receive two types of responses for a connection request:
                // 1. Response::Error if servic eis not allowed to connect
                // 2. Response::Ok and fd right after it if the hub allows the connection
                // We handle second option here

                let fd = socket.as_raw_fd().recv_fd()?;
                let os_stream = unsafe { OsStream::from_raw_fd(fd) };
                let socket = UnixStream::from_std(os_stream).unwrap();
                Ok(Connection::new(service_name, socket))
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

    async fn handle_method_call(&self, method_name: String, params: Bson) -> Vec<u8> {
        let shared_method = self.methods.write().get_mut(&method_name).cloned();
        if let Some(method) = shared_method {
            match method.write().notify(params).await {
                Ok(response) => response.into_bytes(),
                Err(err) => {
                    let call_error = Response::Error(BusError::MethodCallError(err.to_string()));
                    call_error.bytes()
                }
            }
        } else {
            let call_error = Response::Error(BusError::MethodRegistered);
            call_error.bytes()
        }
    }

    async fn handle_client_message(&mut self, message: messages::Message) -> Option<Response> {
        trace!(
            "Incoming service `{:?}` message: {:?}",
            self.service_name.read(),
            message
        );

        None
    }
}
