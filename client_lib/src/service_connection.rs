use std::error::Error;
use std::fmt::Debug;
use std::sync::Arc;

use bytes::BytesMut;
use log::*;
use parking_lot::RwLock;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::mpsc::{self, Sender};
use tokio::sync::oneshot::Sender as OneSender;

use super::utils;
use common::errors::Error as BusError;
use common::messages::{self, Message, Response};

type Shared<T> = Arc<RwLock<T>>;
type CallbackType = Result<Message, Box<dyn Error + Send + Sync>>;

#[derive(Clone)]
pub struct ServiceConnection {
    connection_service_name: Shared<String>,
    service_tx: Sender<(Message, OneSender<CallbackType>)>,
    task_tx: Sender<(Message, OneSender<CallbackType>)>,
}

impl ServiceConnection {
    pub fn new(
        connection_service_name: String,
        mut socket: UnixStream,
        service_tx: Sender<(Message, OneSender<CallbackType>)>,
    ) -> Self {
        let (task_tx, mut rx) = mpsc::channel(32);

        let mut this = Self {
            connection_service_name: Arc::new(RwLock::new(connection_service_name)),
            service_tx,
            task_tx,
        };
        let result = this.clone();

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
                            let response = this.handle_client_message(message).await;
                            socket.write_all(response.bytes().as_slice()).await.unwrap();
                        }
                    },
                    Some((request, callback_tx)) = rx.recv() => {
                        match request {
                            Message::MethodCall{..} => {
                                callback_tx.send(utils::send_receive_message(&mut socket, request).await).unwrap();
                            },
                            m => { error!("Invalid incoming message for a service: {:?}", m) }
                        };
                    }
                };
            }
        });

        result
    }

    pub async fn call<'de, ParamsType: Serialize, ResponseType: DeserializeOwned>(
        &mut self,
        method_name: &str,
        params: &ParamsType,
    ) -> Result<ResponseType, Box<dyn Error + Sync + Send>> {
        let message = messages::make_call_message(method_name, params);

        let response = utils::call_task(&self.task_tx, message).await;

        match response {
            Ok(Message::Response(Response::Return(data))) => {
                match bson::from_bson::<ResponseType>(data) {
                    Ok(data) => Ok(data),
                    Err(err) => {
                        error!("Can't deserialize method response: {}", err.to_string());
                        Err(Box::new(BusError::InvalidResponse))
                    }
                }
            }
            Ok(Message::Response(Response::Error(err))) => {
                warn!(
                    "Failed to perform a call to `{}::{}`: {}",
                    self.connection_service_name.read(),
                    method_name,
                    err.to_string()
                );
                Err(Box::new(err))
            }
            Ok(r) => {
                error!("Invalid Ok response for a method call: {:?}", r);
                Err(Box::new(BusError::InvalidProtocol))
            }
            Err(e) => {
                error!("Ivalid error response from a method call: {:?}", e);
                Err(e)
            }
        }
    }

    /// Handle incoming message from the bus
    async fn handle_client_message(&mut self, message: messages::Message) -> Message {
        trace!("Incoming client message: {:?}", message);

        if let Ok(response) = utils::call_task(&self.service_tx, message).await {
            response
        } else {
            BusError::Internal.into()
        }
    }
}

impl Debug for ServiceConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Connection to {}", self.connection_service_name.read())
    }
}
