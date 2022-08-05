use std::{
    fmt::Debug,
    os::unix::{net::UnixStream as OsStream, prelude::FromRawFd},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
    time::Duration,
};

use bytes::BytesMut;
use serde::{de::DeserializeOwned, Serialize};
use tokio::{
    net::UnixStream,
    sync::mpsc::{self, Sender},
};

use crate::utils::{self, dummy_tx, TaskChannel};
use karo_bus_common::{
    call_registry::CallRegistry,
    errors::Error as BusError,
    messages::{self, IntoMessage, Message, MessageBody, Response, ServiceMessage},
    net,
};

type Shared<T> = Arc<RwLock<T>>;

/// P2p service connection handle
#[derive(Clone)]
pub struct SimplePeer {
    /// Own service name
    service_name: Shared<String>,
    /// SimplePeer service name
    peer_service_name: Shared<String>,
    /// Sender to forward calls to the service
    service_tx: TaskChannel,
    /// Sender to make calls into the task
    task_tx: Sender<(Message, Sender<Message>)>,
    /// Sender to shutdown peer connection
    shutdown_tx: Sender<()>,
    /// Sender to forward calls to the service. Used to reconnect
    call_registry: CallRegistry,
    /// If connection is alive. Otherwise it's reconnecting
    online: Arc<AtomicBool>,
}

impl SimplePeer {
    /// Create new service handle and start tokio task to handle incoming messages from the peer
    pub(crate) fn new(
        service_name: String,
        peer_service_name: String,
        mut stream: UnixStream,
        service_tx: TaskChannel,
    ) -> Self {
        let (task_tx, mut task_rx) = mpsc::channel(32);
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);

        let mut this = Self {
            service_name: Arc::new(RwLock::new(service_name.clone())),
            peer_service_name: Arc::new(RwLock::new(peer_service_name.clone())),
            service_tx: service_tx.clone(),
            task_tx,
            shutdown_tx,
            call_registry: CallRegistry::new(),
            online: Arc::new(AtomicBool::new(true)),
        };
        let result = this.clone();

        tokio::spawn(async move {
            let mut read_buffer = BytesMut::with_capacity(64);

            loop {
                tokio::select! {
                    // Read incoming message from the peer. This one is tricky: if peer is alive, we always
                    // send Some(message). If disconnected, we send Response::Shutdowm, and after that start
                    // sending None to exclude peer handle from polling. Also, this message is sent when a peer
                    // shutting down gracefully
                    Some(message) = this.read_message(&mut stream, &mut read_buffer) => {
                        if matches!(message.body(), MessageBody::Response(Response::Shutdown(_))) {
                            this.close().await
                        } else {
                            // SimplePeer handle resolves call itself. If message returned, redirect to
                            // the service connection
                            let response = this.handle_peer_message(message).await;

                            if this.write_message(&mut stream, response, dummy_tx()).await.is_none() {
                                this.close().await
                            }
                        }
                    },
                    // Handle method calls
                    Some((request, callback_tx)) = task_rx.recv() => {
                        if this.write_message(&mut stream, request, callback_tx).await.is_none() {
                            this.close().await
                        }
                    },
                    Some(_) = shutdown_rx.recv() => {
                        let shutdown_message = Response::Shutdown("Shutdown".into()).into_message(0);
                        let _ = this.write_message(&mut stream, shutdown_message, dummy_tx()).await;
                        drop(stream);
                        return
                    }
                };
            }
        });

        result
    }

    /// Read incoming messages. [PeerConnection] calls callbacks stored in the [CallRegistry] if message is a call
    /// response. Otherwise will return incoming message to the caller
    pub async fn read_message(
        &mut self,
        socket: &mut UnixStream,
        read_buffer: &mut BytesMut,
    ) -> Option<Message> {
        loop {
            match net::read_message_from_socket_log(socket, read_buffer, false).await {
                Ok(message) => {
                    match message.body() {
                        // If got a response to a call, handle it by call_registry. Otherwise it's
                        // an incoming call. Use [handle_bus_message]
                        MessageBody::Response(_) => {
                            self.call_registry.resolve_log(message, false).await
                        }
                        _ => return Some(message),
                    }
                }
                Err(_) => {
                    eprintln!("Failed to read message from a socket. Trying to reconnect");
                    self.reconnect(socket).await;

                    // Ask service to close connection
                    return Some(Response::Shutdown("Peer socket closed".into()).into_message(0));
                }
            }
        }
    }

    /// Outgoing call, which requires reponse. Method calls, subscriptions and state watch requests.
    /// If function fails to write into the socket, it tries to
    /// reconnect and send message again. Having saved call in the [CallRegistry] will call
    /// request callback eventually
    async fn write_message(
        &mut self,
        socket: &mut UnixStream,
        mut message: Message,
        callback: Sender<Message>,
    ) -> Option<()> {
        while let Err(_) = self
            .call_registry
            .call_log(socket, &mut message, &callback, false)
            .await
        {
            eprintln!("Failed to write message into a socket. Offline");
            return None;
        }

        Some(())
    }

    async fn reconnect(&mut self, socket: &mut UnixStream) {
        if self.online.load(Ordering::Acquire) {
            eprintln!("Trying to reconnect while online");
        }

        self.online.store(false, Ordering::Release);

        loop {
            println!(
                "Reconnect loop {} to {}",
                self.service_name.read().unwrap(),
                self.peer_service_name.read().unwrap()
            );

            let connection_message =
                Message::new_connection(self.peer_service_name.read().unwrap().clone(), true);

            // Request service connection to send connection message
            match utils::call_task(&self.service_tx, connection_message).await {
                Ok(message) => {
                    println!("Received reconnected fd message");

                    match message.body() {
                        // This is a message we should receive if succesfully reconnected
                        MessageBody::ServiceMessage(ServiceMessage::PeerFd(fd)) => {
                            let os_stream = unsafe { OsStream::from_raw_fd(*fd) };
                            let stream = UnixStream::from_std(os_stream).unwrap();
                            *socket = stream;

                            self.online.store(true, Ordering::Release);
                            return;
                        }
                        _ => {}
                    }
                }
                Err(err) => {
                    eprintln!(
                        "Failed to receive response for reconnection request: {}",
                        err.to_string()
                    );
                }
            }

            eprintln!("Failed to reconnect simple client. Scheduling retry");
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    pub fn name(&self) -> String {
        self.service_name.read().unwrap().clone()
    }

    /// Remote method call\
    /// **P** is an argument type. Should be a serializable structure.\
    /// **R** is return type. Should be a deserializable structure
    pub async fn call<P: Serialize, R: DeserializeOwned>(
        &self,
        method_name: &str,
        params: &P,
    ) -> crate::Result<R> {
        // Return error if offline
        if !self.online.load(Ordering::Acquire) {
            return Err(BusError::NotConnected.into());
        }

        let message = Message::new_call(
            self.peer_service_name.read().unwrap().clone(),
            method_name.into(),
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
                        Err(_) => Err(Box::new(BusError::InvalidResponse)),
                    }
                }
                // Got an error from the peer
                MessageBody::Response(Response::Error(err)) => Err(Box::new(err.clone())),
                // Invalid protocol
                _ => Err(Box::new(BusError::InvalidMessage)),
            },
            // Network error
            Err(e) => Err(e),
        }
    }

    /// Handle messages from the peer
    async fn handle_peer_message(&mut self, message: messages::Message) -> Message {
        let response_seq = message.seq();

        match utils::call_task(&self.service_tx, message).await {
            Ok(response) => response,
            Err(_) => BusError::Internal.into_message(response_seq),
        }
    }

    /// Close peer connection
    pub async fn close(&mut self) {
        let self_name = self.peer_service_name.read().unwrap().clone();

        let _ = utils::call_task(
            &self.service_tx,
            Response::Shutdown(self_name.clone()).into_message(0),
        );
    }
}

impl Drop for SimplePeer {
    fn drop(&mut self) {
        let shutdown_tx = self.shutdown_tx.clone();

        tokio::spawn(async move {
            let _ = shutdown_tx.send(()).await;
        });
    }
}

impl Debug for SimplePeer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SimplePeer connection to {}",
            self.peer_service_name.read().unwrap()
        )
    }
}
