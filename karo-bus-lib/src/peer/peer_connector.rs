use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use karo_common_messages::{Message, Response};
use log::*;
use tokio::net::UnixStream;

use karo_common_connection::connector::Connector;
use karo_common_rpc::rpc_sender::RpcSender;

/// Peer connector to request peer connection from the hub
pub struct PeerConnector {
    /// Peer name
    peer_name: String,
    /// Hub writer, which is used to connect to a peer
    hub_writer: RpcSender,
    /// If we need to reconnect if connection dropped.
    /// We reconnect only if this is an outgoing connection.
    /// This vaue can change, because we can start subscribing to a service, which
    /// previously initiated connection.
    reconnect: Arc<AtomicBool>,
}

impl PeerConnector {
    pub fn new(peer_name: String, hub_writer: RpcSender, reconnect: Arc<AtomicBool>) -> Self {
        Self {
            peer_name,
            hub_writer,
            reconnect,
        }
    }
}

#[async_trait]
impl Connector for PeerConnector {
    async fn connect(&self) -> Result<UnixStream> {
        info!("Connecting to a peer `{}`", self.peer_name);

        loop {
            debug!("Trying to reconnect to `{}`", self.peer_name);

            let connection_message = Message::new_connection(self.peer_name.clone(), true);

            // Request service connection to send connection message
            match self.hub_writer.call(connection_message.into()).await {
                Ok(receiver) => {
                    let Some(message) = receiver.recv().await else {
                        warn!("Connection to the hub closed. Failed to reconnect to the peer {}", self.peer_name);
                        return Err(anyhow!("Hub connection closed"));
                    };

                    let message_body: Message = message
                        .body()
                        .clone()
                        .try_into()
                        .context("Failed to deserialize incoming message")?;

                    match message_body {
                        // This is the message we should receive if succesfully reconnected
                        Message::Response(Response::Ok) => {
                            // Check if we've receive peer fd
                            if let Some(stream) = message.take_fd() {
                                info!("Succesfully reconnected to `{}`", self.peer_name);
                                return Ok(stream);
                            } else {
                                error!("Hub didn't send us a descriptor after Ok response");
                                return Err(anyhow!("Didn't receive a socket from the hub"));
                            }
                        }
                        m => {
                            // If failed to resubscribe, try to reconnect again
                            error!("Invalid reconnection response: {:?}", m);
                            return Err(anyhow!("Invalid message from the hub: {}", m));
                        }
                    }
                }
                Err(err) => {
                    warn!(
                        "Failed to reconnect `{}` peer: {}. Scheduling retry",
                        self.peer_name,
                        err.to_string()
                    );
                }
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}
