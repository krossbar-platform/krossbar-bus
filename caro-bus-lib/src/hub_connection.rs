use std::sync::Arc;
use std::time::Duration;
use std::{io::Result as IoResult, os::unix::prelude::RawFd};

use async_recursion::async_recursion;
use bytes::BytesMut;
use caro_bus_common::{
    self as common,
    call_registry::CallRegistry,
    errors::Error as BusError,
    messages::{IntoMessage, Message, MessageBody, Response, ServiceMessage},
    monitor::{MonitorMessage, MonitorMessageDirection, MONITOR_METHOD},
    net,
};
use log::*;
use tokio::{
    io::AsyncWriteExt,
    net::UnixStream,
    sync::{mpsc::Sender, RwLock as TokioRwLock},
};
use tokio_send_fd::SendFd;

use crate::peer_connection::PeerConnection;
use crate::utils::dummy_tx;

const RECONNECT_RETRY_PERIOD: Duration = Duration::from_secs(1);

/// Connection state
#[derive(Eq, PartialEq)]
enum State {
    Open,
    Closed,
    Reconnecting,
}

pub(crate) struct HubConnection {
    /// Own service name
    service_name: String,
    /// Unix domain socket connected to the hub
    socket: UnixStream,
    /// Buffer to read incoming messages
    read_buffer: BytesMut,
    /// Registry to make calls and send responses to proper callers
    /// For hub it's registration and connection requests
    call_registry: CallRegistry,
    ///  Connection state
    state: State,
    /// Monitor connection if connected
    monitor: Option<Arc<TokioRwLock<PeerConnection>>>,
}

/// Hub connection, which handles all network requests and responses
impl HubConnection {
    fn new(service_name: &str, socket: UnixStream) -> Self {
        Self {
            service_name: service_name.into(),
            socket,
            read_buffer: BytesMut::with_capacity(64),
            call_registry: CallRegistry::new(),
            state: State::Open,
            monitor: None,
        }
    }

    /// Perform hub connection and registration
    pub async fn connect(service_name: &str) -> crate::Result<Self> {
        info!("Connecting to a hub socket");

        let socket = UnixStream::connect(common::get_hub_socket_path()).await?;

        let mut connection = HubConnection::new(service_name, socket);
        connection.register().await?;
        Ok(connection)
    }

    #[async_recursion]
    async fn reconnect(&mut self) {
        loop {
            // Try to reconnect to the hub. I fails, we just retry
            match UnixStream::connect(common::get_hub_socket_path()).await {
                Ok(socket) => {
                    info!("Succesfully reconnected to the hub");
                    self.socket = socket;

                    // Try to connect and register
                    match self.register().await {
                        Ok(_) => return,
                        Err(err) => {
                            error!(
                                "Failed to reconnect service `{}`: {}",
                                self.service_name,
                                err.to_string()
                            );

                            tokio::time::sleep(RECONNECT_RETRY_PERIOD).await;
                            continue;
                        }
                    }
                }
                Err(_) => {
                    debug!("Failed to reconnect to a hub socket. Will try again");

                    tokio::time::sleep(RECONNECT_RETRY_PERIOD).await;
                    continue;
                }
            }
        }
    }

    /// Send registration request
    /// This method is a blocking call from the library workflow perspectire: we can't make any hub
    /// calls without previous registration
    async fn register(&mut self) -> crate::Result<()> {
        // First connect to a socket
        let self_name = self.service_name.clone();
        debug!("Performing service `{}` registration", self_name);

        // Make a message and send to the hub
        let message = Message::new_registration(self_name.clone());

        self.write_message(message, dummy_tx()).await;

        // Wait for hub response
        match self.read_message().await.unwrap().body() {
            MessageBody::Response(Response::Ok) => {
                info!("Succesfully registered service as `{}`", self_name);

                Ok(())
            }
            MessageBody::Response(Response::Error(err)) => {
                warn!("Failed to register service as `{}`: {}", self_name, err);
                Err(Box::new(err.clone()))
            }
            m => {
                error!("Invalid response from the hub: {:?}", m);
                Err(Box::new(BusError::InvalidMessage))
            }
        }
    }

    /// Read incoming messages. [HubConnection] calls callbacks stored in the [CallRegistry] if message is a call
    /// response. Otherwise will return incoming message to the caller
    pub async fn read_message(&mut self) -> Option<Message> {
        if self.state == State::Closed {
            return None;
        }

        loop {
            match net::read_message_from_socket(&mut self.socket, &mut self.read_buffer).await {
                Ok(message) => {
                    self.send_monitor_message(&message, MonitorMessageDirection::Incoming)
                        .await;

                    match message.body() {
                        // Incoming connection request. Connection socket FD will be coming next
                        MessageBody::ServiceMessage(ServiceMessage::IncomingPeerFd {
                            peer_service_name,
                        }) => {
                            trace!("Incoming file descriptor for a peer: {}", peer_service_name);

                            // If we have a call, use special service message to return file descriptor
                            // The caller can do whatever he wants (connection requests registers new client,
                            // reconnection requests just updates its own socket with incoming one). If hub initiates
                            // connection, we return the message to the `Bus`, which will instantiate new [Peer]
                            // connections
                            if self.call_registry.has_call(message.seq()) {
                                let fd = match self.recv_fd().await {
                                    Ok(fd) => fd,
                                    Err(err) => {
                                        error!("Failed to receive peer fd: {}", err.to_string());
                                        return None;
                                    }
                                };

                                self.call_registry
                                    .resolve(ServiceMessage::PeerFd(fd).into_message(message.seq()))
                                    .await;

                                return None;
                            } else {
                                return Some(message);
                            }
                        }
                        // If got a response to a call, handle it by call_registry. Otherwise it's
                        // an incoming call. Use [handle_bus_message]
                        MessageBody::Response(_) => {
                            if self.call_registry.has_call(message.seq()) {
                                self.call_registry.resolve(message).await;
                                return None;
                            } else {
                                return Some(message);
                            }
                        }
                        _ => return Some(message),
                    }
                }
                Err(err) => {
                    warn!(
                        "Error reading from hub socket: {}. Trying to reconnect",
                        err.to_string()
                    );

                    self.state = State::Reconnecting;
                    self.reconnect().await;
                }
            }
        }
    }

    pub async fn recv_fd(&mut self) -> IoResult<RawFd> {
        self.socket.recv_fd().await
    }

    /// Write outgoing message
    pub async fn write_message(&mut self, message: Message, callback: Sender<Message>) {
        match message.body() {
            MessageBody::ServiceMessage(ServiceMessage::Register { .. }) => {
                self.write_reconnect(message, callback).await
            }
            MessageBody::ServiceMessage(ServiceMessage::Connect { .. }) => {
                self.call_reconnect(message, callback).await
            }
            MessageBody::MethodCall { .. } => self.call_reconnect(message, callback).await,
            MessageBody::Response(_) => self.write_reconnect(message, callback).await,
            m => {
                error!("Invalid incoming message for the hub: {:?}", m);
            }
        };
    }

    /// Outgoing message, which doesn't require reponse. Mostly signals emissions or state changes.
    /// If function fails to write into the socket, it tries to
    /// reconnect, but drops response mpsc sender, because even if reconnected, new peer
    /// will try to resibscribe, and current subscription becomes invalid
    async fn write_reconnect(&mut self, message: Message, callback: Sender<Message>) {
        while let Err(err) = self.socket.write_all(message.bytes().as_slice()).await {
            warn!(
                "Failed to send write a message to the peer `{}`: {}",
                self.service_name,
                err.to_string()
            );

            // Drops callback, because current subscription became invalid. See function comment
            drop(callback);

            self.state = State::Reconnecting;
            self.reconnect().await;
            return;
        }

        let _ = callback.send(Response::Ok.into_message(0)).await;

        self.send_monitor_message(&message, MonitorMessageDirection::Outgoing)
            .await;
    }

    /// Outgoing call, which requires reponse. Registration and connection requests.
    /// If function fails to write into the socket, it tries to
    /// reconnect and send message again. Having saved call in the [CallRegistry] will call
    /// request callback eventually
    async fn call_reconnect(&mut self, mut message: Message, callback: Sender<Message>) {
        while let Err(err) = self
            .call_registry
            .call(&mut self.socket, &mut message, &callback)
            .await
        {
            warn!(
                "Failed to make a call to the hub: {}. Reconnecting",
                err.to_string()
            );

            self.state = State::Reconnecting;
            self.reconnect().await;
        }

        self.send_monitor_message(&message, MonitorMessageDirection::Outgoing)
            .await;
    }

    /// Send message to the Caro monitor if connected
    async fn send_monitor_message(
        &mut self,
        message: &Message,
        direction: MonitorMessageDirection,
    ) {
        let (sender, receiver) = match direction {
            MonitorMessageDirection::Outgoing => (self.service_name.clone(), "hub".into()),
            MonitorMessageDirection::Incoming => ("hub".into(), self.service_name.clone()),
        };

        if let Some(ref monitor) = self.monitor {
            // First we make monitor message, which will be sent as method call parameter...
            let monitor_message = MonitorMessage::new(sender, receiver, &message, direction);

            // ..And to call monitor method, we need
            let method_call = Message::new_call(
                self.service_name.clone(),
                MONITOR_METHOD.into(),
                &monitor_message,
            );

            trace!("Sending monitor message: {:?}", message);

            if monitor
                .write()
                .await
                .write_message(method_call, dummy_tx())
                .await
                .is_none()
            {
                debug!("Monitor disconnected");
                self.monitor = None;
            }
        }
    }

    pub fn set_monitor(&mut self, monitor_connection: Arc<TokioRwLock<PeerConnection>>) {
        debug!("Incoming monitor connection for the hub");

        self.monitor = Some(monitor_connection);
    }
}
