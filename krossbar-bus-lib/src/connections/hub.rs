use anyhow::Result;

use krossbar_common_rpc::{rpc_connection::RpcConnection, rpc_sender::RpcSender};

use super::hub_connector::HubConnector;

pub(crate) struct Hub {
    /// Own service name
    service_name: String,
    /// Sender into the socket to return to the client
    hub_sender: RpcSender,
    /// RpcConnection
    hub_connection: RpcConnection,
}

/// Hub connection, which handles all network requests and responses
impl Hub {
    pub async fn new(service_name: &str) -> Result<RpcConnection> {
        // Peer connector, which will connect to the peer if this is and outgoing connection
        let connector = Box::new(HubConnector::new(service_name.into()));

        // Rpc connection
        RpcConnection::new(connector).await
    }
}
