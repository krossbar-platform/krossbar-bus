use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use krossbar_common_messages::{Message, Response};
use log::*;
use tokio::net::UnixStream;

use krossbar_common_rpc::{rpc_connector::RpcConnector, rpc_sender::RpcSender};

/// Peer connector to request peer connection from the hub
pub struct PeerConnector {
    /// Peer name
    peer_name: String,
    /// Hub writer, which is used to connect to a peer
    hub_writer: RpcSender,
    /// If it is an incoming connection, we've laready got a stream from the peer
    incoming_stream: Option<UnixStream>,
    /// If we need to reconnect if connection dropped.
    /// We reconnect only if this is an outgoing connection.
    /// This vaue can change, because we can start subscribing to a service, which
    /// previously initiated connection.
    reconnect: Arc<AtomicBool>,
}

impl PeerConnector {
    pub fn new(
        peer_name: String,
        hub_writer: RpcSender,
        incoming_stream: Option<UnixStream>,
        reconnect: Arc<AtomicBool>,
    ) -> Self {
        Self {
            peer_name,
            hub_writer,
            incoming_stream,
            reconnect,
        }
    }
}

#[async_trait]
impl RpcConnector for PeerConnector {
    async fn connect(&self) -> Result<UnixStream> {
        // Incoming connection already has a stream received from the hub
        if let Some(incoming_stream) = self.incoming_stream.take() {
            info!(
                "Creating RPC conenction from an incoming peer stream `{}`",
                self.peer_name
            );
            return Ok(incoming_stream);
        }

        if !self.reconnect.load(Ordering::Release) {
            return Err(anyhow!("Refused to reconnect incoming connections"));
        }

        info!("Connecting to a peer `{}`", self.peer_name);

        debug!("Trying to reconnect to the peer `{}`", self.peer_name);

        let connection_message = Message::new_connection(self.peer_name.clone(), true);

        // Request service connection to send connection message
        let message = self
            .hub_writer
            .call(&connection_message)
            .await
            .context(format!(
                "Connection to the hub closed. Failed to reconnect to the peer {}",
                self.peer_name
            ))?;

        let message_body: Message = message.body();

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
}
