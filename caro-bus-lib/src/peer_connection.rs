use std::{error::Error, fmt::Debug, sync::Arc};

use bytes::BytesMut;
use log::*;
use parking_lot::RwLock;
use serde::{de::DeserializeOwned, Serialize};
use tokio::{
    io::AsyncWriteExt,
    net::UnixStream,
    sync::{
        mpsc::{self, Receiver, Sender},
        watch::Receiver as WatchReceiver,
    },
};

use super::utils::{self, TaskChannel};
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
    /// Sender to forward calls to the service
    service_tx: TaskChannel,
    /// Sender to make calls into the task
    task_tx: TaskChannel,
    /// Sender to shutdown peer connection
    shutdown_tx: Sender<()>,
    /// Registry to make calls and send responses to proper callers
    /// Includes methods, signals, and states
    call_registry: CallRegistry,
}

impl PeerConnection {
    /// Create new service handle and start tokio task to handle incoming messages from the peer
    pub fn new(peer_service_name: String, mut socket: UnixStream, service_tx: TaskChannel) -> Self {
        let (task_tx, mut task_rx) = mpsc::channel(32);
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);

        let mut this = Self {
            peer_service_name: Arc::new(RwLock::new(peer_service_name)),
            service_tx,
            task_tx,
            shutdown_tx,
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
                                    MessageBody::Response(_) => this.call_registry.resolve(message).await,
                                    _ => {
                                        let response = this.handle_peer_message(message).await;

                                        if let Err(_) = socket.write_all(response.bytes().as_slice()).await {
                                            warn!("Failed to write peer `{}` response. Shutting him down", this.peer_service_name.read());

                                            this.close_with_socket(&mut socket).await;
                                            return
                                        }
                                    }
                                }
                            },
                            Err(_) => {
                                let peer_name = this.peer_service_name.read().clone();
                                warn!("Failed to read peer `{}` message. Shutting him down", peer_name);

                                this.close_with_socket(&mut socket).await;
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
                                    this.close_with_socket(&mut socket).await;
                                    return
                                }
                            },
                            MessageBody::SignalSubscription{ .. } => {
                                if let Err(_) = this.call_registry.call(&mut socket, request, callback_tx).await {
                                    this.close_with_socket(&mut socket).await;
                                    return
                                }
                            },
                            MessageBody::Response(Response::Signal(_)) => {
                                if let Err(_) = socket.write_all(request.bytes().as_slice()).await {
                                    warn!("Failed to write peer `{}` signal. Shutting him down", this.peer_service_name.read());

                                    this.close_with_socket(&mut socket).await;
                                    return
                                }

                            }
                            m => { error!("Invalid incoming message for a service: {:?}", m) }
                        };
                    },
                    Some(_) = shutdown_rx.recv() => {
                        drop(socket);
                        return
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
        let message = Message::new_call(
            self.peer_service_name.read().clone(),
            method_name.clone(),
            params,
        );

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

    /// Remote signal subscription
    pub async fn subscribe<T: DeserializeOwned>(
        &mut self,
        signal_name: &String,
        callback: impl Fn(&T) + Send + 'static,
    ) -> Result<(), Box<dyn Error + Sync + Send>> {
        let message =
            Message::new_subscription(self.peer_service_name.read().clone(), signal_name.clone());

        // This is a tricky one. First we use channel to read subscription status, and after that
        // for incomin signal emission
        let (tx, mut rx) = mpsc::channel(10);

        // Send subscription request
        self.task_tx.send((message, tx)).await?;

        let response = rx.recv().await.unwrap();

        match response.body() {
            // Succesfully performed remote method call
            MessageBody::Response(Response::Ok) => {
                debug!("Succesfully subscribed to the signal {}", signal_name);

                PeerConnection::start_subscription_receiving_task(signal_name, rx, callback);

                Ok(())
            }
            // Got an error from the peer
            MessageBody::Response(Response::Error(err)) => {
                warn!(
                    "Failed to subscribe to `{}::{}`: {}",
                    self.peer_service_name.read(),
                    signal_name,
                    err.to_string()
                );
                Err(Box::new(err.clone()))
            }
            // Invalid protocol
            r => {
                error!("Invalid Ok response for a method call: {:?}", r);
                Err(Box::new(BusError::InvalidProtocol))
            }
        }
    }

    /// Start task to receive signals emission and calling user callback
    fn start_subscription_receiving_task<T: DeserializeOwned>(
        signal_name: &String,
        mut receiver: Receiver<Message>,
        callback: impl Fn(&T) + Send + 'static,
    ) {
        let signal_name = signal_name.clone();

        // Start listening to signal emissions
        tokio::spawn(async move {
            loop {
                match receiver.recv().await {
                    Some(message) => {
                        match message.body() {
                            MessageBody::Response(Response::Signal(value)) => {
                                match bson::from_bson::<T>(value.clone()) {
                                    Ok(value) => {
                                        // Call callback
                                        callback(&value);
                                    }
                                    Err(err) => {
                                        error!(
                                            "Failed to deserialize signal value: {}",
                                            err.to_string()
                                        );
                                    }
                                }
                            }
                            m => {
                                error!("Invalid message inside signal handling code: {:?}", m);
                            }
                        }
                    }
                    None => {
                        error!(
                            "Failed to listen to signal {} subscription. Cancelling",
                            signal_name
                        );
                        return;
                    }
                }
            }
        });
    }

    /// Start subscription task, which polls signal Receiver and sends peer message
    /// if emited
    pub(crate) fn start_signal_sending_task(&self, mut signal_receiver: WatchReceiver<Message>) {
        let self_tx = self.task_tx.clone();

        tokio::spawn(async move {
            loop {
                // Wait for signal emission
                match signal_receiver.changed().await {
                    Ok(_) => {
                        let message = (*signal_receiver.borrow()).clone();
                        // Call self task to send signal message
                        let _ = utils::call_task(&self_tx, message).await;
                    }
                    Err(err) => {
                        error!("Signal receiver error: {:?}", err);
                        return;
                    }
                }
            }
        });
    }

    /// Handle messages from the peer
    async fn handle_peer_message(&mut self, message: messages::Message) -> Message {
        trace!("Incoming client message: {:?}", message);

        let response_seq = message.seq();

        match utils::call_task(&self.service_tx, message).await {
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
        let self_name = self.peer_service_name.read().clone();
        debug!(
            "Shutting down peer connection to `{:?}`",
            self.peer_service_name.read()
        );

        let _ = utils::call_task(
            &self.service_tx,
            Response::Shutdown(self_name.clone()).into_message(0),
        );
    }

    /// Closes socket end shuts service down. We need this in case of socket IO errors.
    /// In this case socket will be always readable with zero bytes received
    async fn close_with_socket(&mut self, socket: &mut UnixStream) {
        drop(socket);
        self.close().await;
    }
}

impl Drop for PeerConnection {
    fn drop(&mut self) {
        let shutdown_tx = self.shutdown_tx.clone();

        tokio::spawn(async move {
            let _ = shutdown_tx.send(()).await;
        });
    }
}

impl Debug for PeerConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Peer connection to {}", self.peer_service_name.read())
    }
}
