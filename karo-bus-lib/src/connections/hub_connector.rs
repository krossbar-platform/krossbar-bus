use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use karo_common_rpc::{rpc_connector::RpcConnector, rpc_sender::RpcSender};
use log::*;
use tokio::{net::UnixStream, time::sleep};

use karo_common_messages::{Message, Response};

/// Peer connector to request peer connection from the hub
pub struct HubConnector {
    /// Peer name
    service_name: String,
}

impl HubConnector {
    pub fn new(service_name: String) -> Self {
        Self { service_name }
    }
}

#[async_trait]
impl RpcConnector for HubConnector {
    async fn connect(&self) -> Result<UnixStream> {
        info!("Connecting to a hub socket");

        loop {
            match UnixStream::connect(karo_bus_common::get_hub_socket_path()).await {
                Ok(socket) => return Ok(socket),
                Err(err) => {
                    warn!(
                        "Failed to reconnect to the hub: {}. Retrying",
                        err.to_string()
                    );

                    sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }

    /// Send registration request
    /// This method is a blocking call from the library workflow perspectire: we can't make any hub
    /// calls without previous registration
    async fn on_connected(&self, sender: &mut RpcSender) -> Result<()> {
        let self_name = self.service_name.clone();
        debug!("Performing service `{}` registration", self_name);

        // Make a message and send to the hub
        let message = Message::new_registration(self_name.clone());

        match sender.call(&message).await?.body() {
            // Failed to register the service
            Message::Response(Response::Error(error)) => Err(error.into()),
            _ => Ok(()),
        }
    }
}
