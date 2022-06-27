use std::time::Duration;

use bytes::BytesMut;
use caro_bus_common::{
    call_registry::CallRegistry,
    messages::{IntoMessage, Message, MessageBody, Response},
    net,
};
use log::*;
use tokio::{io::AsyncWriteExt, net::UnixStream, sync::mpsc::Sender};

use crate::utils::TaskChannel;

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
}

impl Peer {
    pub fn new(name: String, socket: UnixStream, service_tx: TaskChannel) -> Self {
        Self {
            name,
            socket,
            service_tx,
            read_buffer: BytesMut::with_capacity(64),
            call_registry: CallRegistry::new(),
        }
    }

    pub async fn read_message(&mut self) -> Message {
        loop {
            match net::read_message_from_socket(&mut self.socket, &mut self.read_buffer).await {
                Ok(message) => {
                    match message.body() {
                        // If got a response to a call, handle it by call_registry. Otherwise it's
                        // an incoming call. Use [handle_bus_message]
                        MessageBody::Response(_) => self.call_registry.resolve(message).await,
                        _ => return message,
                    }
                }
                Err(err) => {
                    warn!(
                        "Error reading from peer socket: {}. Trying to reconnect",
                        err.to_string()
                    );

                    self.reconnect().await;
                    continue;
                }
            }
        }
    }

    pub async fn write_message(&mut self, message: Message, callback: Sender<Message>) {
        match message.body() {
            MessageBody::MethodCall { .. } => {
                self.call_reconnect(message, callback).await;
            }
            MessageBody::SignalSubscription { .. } => {
                self.call_reconnect(message, callback).await;
            }
            MessageBody::StateSubscription { .. } => {
                self.call_reconnect(message, callback).await;
            }
            MessageBody::Response(Response::Signal(_)) => {
                self.write_reconnect(message, callback).await;
            }
            MessageBody::Response(Response::StateChanged(_)) => {
                self.write_reconnect(message, callback).await;
            }
            m => {
                error!("Invalid incoming message for a service: {:?}", m)
            }
        };
    }

    /// Signal or state change. If function fails to write into the socket, it tries to
    /// reconnect, but drops response mpsc sender, because even if reconnected, new peer
    /// will try to resibscribe, and current subscription becomes invalid
    async fn write_reconnect(&mut self, message: Message, callback: Sender<Message>) {
        while let Err(_) = self.socket.write_all(message.bytes().as_slice()).await {
            warn!(
                "Failed to send write a message to the peer `{}`. Reconnecting",
                self.name
            );

            // Drops callback, because current subscription became invalid. See function comment
            drop(callback);
            self.reconnect().await;
            return;
        }

        let _ = callback.send(Response::Ok.into_message(0)).await;
    }

    async fn call_reconnect(&mut self, mut message: Message, callback: Sender<Message>) {
        while let Err(_) = self
            .call_registry
            .call(&mut self.socket, &mut message, &callback)
            .await
        {
            warn!(
                "Failed to make a call to the peer `{}`. Reconnecting",
                self.name
            );
            self.reconnect().await;
        }
    }

    // TODO: Handle non-existing services
    async fn reconnect(&mut self) {
        let connection_message = Message::new_connection(self.name.clone());

        tokio::time::sleep(Duration::from_secs(1)).await
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        drop(&self.socket)
    }
}
