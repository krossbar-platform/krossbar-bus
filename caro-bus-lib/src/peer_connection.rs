use std::{error::Error, fmt::Debug, sync::Arc};

use bytes::BytesMut;
use log::*;
use parking_lot::RwLock;
use serde::{de::DeserializeOwned, Serialize};
use tokio::{
    io::AsyncWriteExt,
    net::UnixStream,
    sync::{
        mpsc::{self, Sender},
        oneshot::Sender as OneSender,
    },
};

use super::{utils, BusConnection, CallbackType};
use caro_bus_common::{
    errors::Error as BusError,
    messages::{self, Message, Response},
    net,
};

type Shared<T> = Arc<RwLock<T>>;

/// P2p service connection handle
#[derive(Clone)]
pub struct PeerConnection {
    /// Peer service name
    peer_service_name: Shared<String>,
    /// Bus connection send incoming messages from the peer
    bus: BusConnection,
    /// Sender to make calls into the task
    task_tx: Sender<(Message, OneSender<CallbackType>)>,
}

impl PeerConnection {
    /// Create new service handle and start tokio task to handle incoming messages from the peer
    pub fn new(peer_service_name: String, mut socket: UnixStream, bus: BusConnection) -> Self {
        let (task_tx, mut task_rx) = mpsc::channel(32);

        let mut this = Self {
            peer_service_name: Arc::new(RwLock::new(peer_service_name)),
            bus,
            task_tx,
        };
        let result = this.clone();

        tokio::spawn(async move {
            let mut bytes = BytesMut::with_capacity(64);

            loop {
                tokio::select! {
                    // Read incoming message from the peer
                    read_result = net::read_message(&mut socket, &mut bytes) => {
                        match read_result {
                            Ok(message) => {
                                let response = this.handle_peer_message(message).await;

                                if let Err(_) = socket.write_all(response.bytes().as_slice()).await {
                                    let peer_name = this.peer_service_name.read().clone();
                                    error!("Failed to write peer `{}` response. Shutting him down", peer_name);

                                    this.bus.remove_peer(peer_name);
                                    drop(socket);
                                    return
                                }
                            },
                            Err(_) => {
                                let peer_name = this.peer_service_name.read().clone();
                                error!("Failed to read peer `{}` message. Shutting him down", peer_name);

                                this.bus.remove_peer(peer_name);
                                drop(socket);
                                return
                            }
                        }
                    },
                    // Handle method calls
                    Some((request, callback_tx)) = task_rx.recv() => {
                        trace!("Peer task message: {:?}", request);

                        match request {
                            Message::MethodCall{..} => {
                                callback_tx.send(utils::send_receive_message(&mut socket, request).await).unwrap();
                            },
                            Message::Response(Response::Shutdown) => {
                                //let peer_name = this.peer_service_name.read().clone();
                                //error!("Peer connection `{}` received shutdown request",  peer_name);
                                callback_tx.send(Ok(Response::Ok.into())).unwrap();

                                drop(socket);
                                return
                            }
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

        match self.bus.handle_peer_message(message).await {
            Ok(response) => response,
            Err(err) => {
                warn!(
                    "Error return as a result of peer message handling: {}",
                    err.to_string()
                );

                BusError::InvalidProtocol.into()
            }
        }
    }

    pub async fn close(&mut self) {
        debug!(
            "Shutting down peer connection to `{:?}`",
            self.peer_service_name.read()
        );

        utils::call_task(&self.task_tx, Response::Shutdown.into())
            .await
            .unwrap();
    }
}

impl Debug for PeerConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Peer connection to {}", self.peer_service_name.read())
    }
}
