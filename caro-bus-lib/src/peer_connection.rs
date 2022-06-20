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

use super::{utils, BusConnection};
use caro_bus_common::{
    call_registry::CallRegistry,
    errors::Error as BusError,
    messages::{self, IntoMessage, Message, MessageBody, Response},
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
    task_tx: Sender<(Message, OneSender<Message>)>,
    /// Registry to make calls and send responses to proper callers
    call_registry: CallRegistry,
}

impl PeerConnection {
    /// Create new service handle and start tokio task to handle incoming messages from the peer
    pub fn new(peer_service_name: String, mut socket: UnixStream, bus: BusConnection) -> Self {
        let (task_tx, mut task_rx) = mpsc::channel(32);

        let mut this = Self {
            peer_service_name: Arc::new(RwLock::new(peer_service_name)),
            bus,
            task_tx,
            call_registry: CallRegistry::new(),
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
                                match message.body() {
                                    // If got a response to a call, handle it by call_registry. Otherwise it's
                                    // an incoming call. Use [handle_bus_message]
                                    MessageBody::Response(_) => this.call_registry.resolve(message),
                                    _ => {
                                        let response = this.handle_peer_message(message).await;

                                        if let Err(_) = socket.write_all(response.bytes().as_slice()).await {
                                            let peer_name = this.peer_service_name.read().clone();
                                            warn!("Failed to write peer `{}` response. Shutting him down", peer_name);

                                            this.bus.remove_peer(peer_name);
                                            drop(socket);
                                            return
                                        }
                                    }
                                }
                            },
                            Err(_) => {
                                let peer_name = this.peer_service_name.read().clone();
                                warn!("Failed to read peer `{}` message. Shutting him down", peer_name);

                                this.bus.remove_peer(peer_name);
                                drop(socket);
                                return
                            }
                        }
                    },
                    // Handle method calls
                    Some((request, callback_tx)) = task_rx.recv() => {
                        trace!("Peer task message: {:?}", request);

                        match request.body() {
                            MessageBody::MethodCall{ .. } => {
                                if let Err(_)  = this.call_registry.call(&mut socket, request, callback_tx).await {
                                    let peer_name = this.peer_service_name.read().clone();

                                    this.bus.remove_peer(peer_name);
                                    drop(socket);
                                    return
                                }
                            },
                            MessageBody::Response(Response::Shutdown) => {
                                let peer_name = this.peer_service_name.read().clone();
                                info!("Peer connection `{}` received shutdown request",  peer_name);
                                callback_tx.send(Response::Ok.into_message(999)).unwrap();

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
        let message = MessageBody::MethodCall {
            method_name: method_name.clone(),
            params: bson::to_bson(params).unwrap(),
        }
        .into_message(999);

        // Send method call request
        let response = utils::call_task(&self.task_tx, message).await;

        match response {
            Ok(message) => match message.body() {
                // Succesfully performed remote method call
                MessageBody::Response(Response::Return(data)) => {
                    match bson::from_bson::<R>(data.clone()) {
                        Ok(data) => Ok(data),
                        Err(err) => {
                            error!("Can't deserialize method response: {}", err.to_string());
                            Err(Box::new(BusError::InvalidResponse))
                        }
                    }
                }
                // Got an error from the peer
                MessageBody::Response(Response::Error(err)) => {
                    warn!(
                        "Failed to perform a call to `{}::{}`: {}",
                        self.peer_service_name.read(),
                        method_name,
                        err.to_string()
                    );
                    Err(Box::new(err.clone()))
                }
                // Invalid protocol
                r => {
                    error!("Invalid Ok response for a method call: {:?}", r);
                    Err(Box::new(BusError::InvalidProtocol))
                }
            },
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

        let response_seq = message.seq();

        match self.bus.handle_peer_message(message).await {
            Ok(response) => response,
            Err(err) => {
                warn!(
                    "Error return as a result of peer message handling: {}",
                    err.to_string()
                );

                BusError::InvalidProtocol.into_message(response_seq)
            }
        }
    }

    pub async fn close(&mut self) {
        debug!(
            "Shutting down peer connection to `{:?}`",
            self.peer_service_name.read()
        );

        utils::call_task(&self.task_tx, Response::Shutdown.into_message(999))
            .await
            .unwrap();
    }
}

impl Debug for PeerConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Peer connection to {}", self.peer_service_name.read())
    }
}
