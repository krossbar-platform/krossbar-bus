use std::os::unix::{net::UnixStream as OsStream, prelude::FromRawFd};
use std::time::Duration;

use bytes::BytesMut;
use caro_bus_common::{
    call_registry::CallRegistry,
    messages::{IntoMessage, Message, MessageBody, Response, ServiceMessage},
    net,
};
use log::*;
use tokio::{io::AsyncWriteExt, net::UnixStream, sync::mpsc::Sender};

use crate::utils::{self, TaskChannel};

#[derive(Eq, PartialEq)]
enum State {
    Open,
    Closed,
    Reconnecting,
}

pub struct Peer {
    name: String,
    // Peer socket
    socket: UnixStream,
    /// Sender to forward calls to the service. Used to reconnect
    service_tx: TaskChannel,
    /// Buffer to read incoming messages
    read_buffer: BytesMut,
    /// Registry to make calls and send responses to proper callers
    /// Includes methods, signals, and states
    call_registry: CallRegistry,
    /// Only outgoing connections will reconnect
    outgoing: bool,
    /// Peer state
    state: State,
    /// Active subscriptions. Used after reconnection to resubscribe
    subscriptions: Vec<(Message, Sender<Message>)>,
}

impl Peer {
    pub fn new(name: String, socket: UnixStream, service_tx: TaskChannel, outgoing: bool) -> Self {
        Self {
            name,
            socket,
            service_tx,
            read_buffer: BytesMut::with_capacity(64),
            call_registry: CallRegistry::new(),
            outgoing,
            state: State::Open,
            subscriptions: Vec::new(),
        }
    }

    pub async fn read_message(&mut self) -> Option<Message> {
        if self.state == State::Closed {
            return None;
        }

        loop {
            match net::read_message_from_socket(&mut self.socket, &mut self.read_buffer).await {
                Ok(message) => {
                    match message.body() {
                        // If got a response to a call, handle it by call_registry. Otherwise it's
                        // an incoming call. Use [handle_bus_message]
                        MessageBody::Response(_) => self.call_registry.resolve(message).await,
                        _ => return Some(message),
                    }
                }
                Err(err) => {
                    warn!(
                        "Error reading from peer socket: {}. Trying to reconnect",
                        err.to_string()
                    );

                    if self.outgoing {
                        self.state = State::Reconnecting;
                        self.reconnect().await;
                        continue;
                    }

                    self.state = State::Closed;
                    // Ask service to close connection
                    return Some(Response::Shutdown("Peer socket closed".into()).into_message(0));
                }
            }
        }
    }

    pub async fn write_message(
        &mut self,
        message: Message,
        callback: Sender<Message>,
    ) -> Option<()> {
        match message.body() {
            MessageBody::MethodCall { .. } => self.call_reconnect(message, callback).await,
            MessageBody::SignalSubscription { .. } => {
                // Save subscription in case we need to reconnect
                if self.outgoing {
                    self.subscriptions.push((message.clone(), callback.clone()));
                }

                self.call_reconnect(message, callback).await
            }
            MessageBody::StateSubscription { .. } => {
                // Save subscription in case we need to reconnect
                if self.outgoing {
                    self.subscriptions.push((message.clone(), callback.clone()));
                }

                self.call_reconnect(message, callback).await
            }
            MessageBody::Response(_) => self.write_reconnect(message, callback).await,
            m => {
                error!("Invalid incoming message for a service: {:?}", m);
                None
            }
        }
    }

    pub async fn shutdown(&mut self) {
        trace!("Shutting down peer `{}` handle", self.name);

        let _ = self.socket.shutdown().await;
        drop(&self.socket);
    }

    /// Signal or state change. If function fails to write into the socket, it tries to
    /// reconnect, but drops response mpsc sender, because even if reconnected, new peer
    /// will try to resibscribe, and current subscription becomes invalid
    async fn write_reconnect(&mut self, message: Message, callback: Sender<Message>) -> Option<()> {
        while let Err(err) = self.socket.write_all(message.bytes().as_slice()).await {
            warn!(
                "Failed to send write a message to the peer `{}`: {}",
                self.name,
                err.to_string()
            );

            // Drops callback, because current subscription became invalid. See function comment
            drop(callback);

            if self.outgoing {
                self.state = State::Reconnecting;
                self.reconnect().await;
                return Some(());
            } else {
                self.state = State::Closed;
                return None;
            }
        }

        let _ = callback.send(Response::Ok.into_message(0)).await;
        Some(())
    }

    async fn call_reconnect(
        &mut self,
        mut message: Message,
        callback: Sender<Message>,
    ) -> Option<()> {
        while let Err(err) = self
            .call_registry
            .call(&mut self.socket, &mut message, &callback)
            .await
        {
            warn!(
                "Failed to make a call to the peer `{}`: {}",
                self.name,
                err.to_string()
            );

            if self.outgoing {
                self.state = State::Reconnecting;
                self.reconnect().await;
                return Some(());
            } else {
                self.state = State::Closed;
                return None;
            }
        }

        Some(())
    }

    // TODO: Handle non-existing services
    async fn reconnect(&mut self) {
        info!("Incoming request to reconnect to `{}`", self.name);

        loop {
            debug!("Trying to reconnect to `{}`", self.name);

            let connection_message = Message::new_connection(self.name.clone(), true);

            // Request service connection to send connection message
            match utils::call_task(&self.service_tx, connection_message).await {
                Ok(message) => {
                    match message.body() {
                        // This is a message we should receive if succesfully reconnected
                        MessageBody::ServiceMessage(ServiceMessage::PeerFd(fd)) => {
                            let os_stream = unsafe { OsStream::from_raw_fd(*fd) };
                            let stream = UnixStream::from_std(os_stream).unwrap();
                            self.socket = stream;

                            info!("Succesfully reconnected to `{}`", self.name);

                            // If failed to resubscribe, try to reconnect again
                            if !self.resubscribe().await {
                                continue;
                            }

                            return;
                        }
                        m => {
                            debug!("Reconnection response: {:?}", m);
                        }
                    }
                }
                Err(err) => {
                    error!(
                        "Failed to receive response for reconnection request: {}",
                        err.to_string()
                    );
                }
            }

            warn!("Failed to reconnect `{}` peer. Scheduling retry", self.name);
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    async fn resubscribe(&mut self) -> bool {
        debug!("Trying to resubscribe to `{}`", self.name);

        for (subscription_message, callback) in self.subscriptions.iter() {
            if let Err(_) = self
                .call_registry
                .call(
                    &mut self.socket,
                    &mut subscription_message.clone(),
                    &callback,
                )
                .await
            {
                return false;
            }
        }

        info!("Succesfully resubscribed to `{}`", self.name);
        true
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        trace!("Peer `{}` handle dropped", self.name);

        drop(&self.socket)
    }
}
