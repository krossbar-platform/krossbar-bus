use std::{error::Error, fmt::Debug, sync::Arc};

use bytes::BytesMut;
use log::*;
use parking_lot::RwLock;
use serde::{de::DeserializeOwned, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
    sync::{
        mpsc::{self, Sender},
        oneshot::Sender as OneSender,
    },
};

use super::{utils, CallbackType};
use caro_bus_common::{
    errors::Error as BusError,
    messages::{self, Message, Response},
};

type Shared<T> = Arc<RwLock<T>>;

/// P2p service connection handle
#[derive(Clone)]
pub struct PeerConnection {
    /// Peer service name
    peer_service_name: Shared<String>,
    /// Bus connection tx to send incoming messages from the peer
    bus_connection_tx: Sender<(Message, OneSender<CallbackType>)>,
    /// Sender to make calls into the task
    task_tx: Sender<(Message, OneSender<CallbackType>)>,
}

impl PeerConnection {
    /// Create new service handle and start tokio task to handle incoming messages from the peer
    pub fn new(
        peer_service_name: String,
        mut socket: UnixStream,
        bus_connection_tx: Sender<(Message, OneSender<CallbackType>)>,
    ) -> Self {
        let (task_tx, mut rx) = mpsc::channel(32);

        let mut this = Self {
            peer_service_name: Arc::new(RwLock::new(peer_service_name)),
            bus_connection_tx,
            task_tx,
        };
        let result = this.clone();

        tokio::spawn(async move {
            let mut bytes = BytesMut::with_capacity(64);

            loop {
                tokio::select! {
                    // Read incoming message from the peer
                    read_result = socket.read_buf(&mut bytes) => {
                        if let Err(err) = read_result {
                            error!("Failed to read from a socket: {}. Hub is down", err.to_string());
                            drop(socket);
                            return
                        }

                        let bytes_read = read_result.unwrap();
                        trace!("Read {} bytes from a peer socket", bytes_read);

                        // Socket closed
                        if bytes_read == 0 {
                            warn!("Peer closed its socket. Shutting down the connection");

                            drop(socket);
                            return
                        }

                        if let Some(message) = messages::parse_buffer(&mut bytes) {
                            let response = this.handle_peer_message(message).await;
                            socket.write_all(response.bytes().as_slice()).await.unwrap();
                        }
                    },
                    // Handle method calls
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

    /// Remote method call
    pub async fn call<P: Serialize, R: DeserializeOwned>(
        &mut self,
        method_name: &String,
        params: &P,
    ) -> Result<R, Box<dyn Error + Sync + Send>> {
        let message = messages::make_call_message(method_name, params);

        // Send method call request
        let response = utils::call_task(&self.task_tx, message).await;

        match response {
            // Succesfully performed remote method call
            Ok(Message::Response(Response::Return(data))) => match bson::from_bson::<R>(data) {
                Ok(data) => Ok(data),
                Err(err) => {
                    error!("Can't deserialize method response: {}", err.to_string());
                    Err(Box::new(BusError::InvalidResponse))
                }
            },
            // Got an error from the peer
            Ok(Message::Response(Response::Error(err))) => {
                warn!(
                    "Failed to perform a call to `{}::{}`: {}",
                    self.peer_service_name.read(),
                    method_name,
                    err.to_string()
                );
                Err(Box::new(err))
            }
            // Invalid protocol
            Ok(r) => {
                error!("Invalid Ok response for a method call: {:?}", r);
                Err(Box::new(BusError::InvalidProtocol))
            }
            // Network error
            Err(e) => {
                error!("Ivalid error response from a method call: {:?}", e);
                Err(e)
            }
        }
    }

    /// Handle messages from the peer
    async fn handle_peer_message(&mut self, message: messages::Message) -> Message {
        trace!("Incoming client message: {:?}", message);

        if let Ok(response) = utils::call_task(&self.bus_connection_tx, message).await {
            response
        } else {
            BusError::Internal.into()
        }
    }
}

impl Debug for PeerConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Peer connection to {}", self.peer_service_name.read())
    }
}
