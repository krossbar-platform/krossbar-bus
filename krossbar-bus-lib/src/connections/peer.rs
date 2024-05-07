use std::{
    fmt::Debug,
    sync::{atomic::AtomicBool, Arc},
};

use anyhow::{Context, Result};
use bson::Bson;
use krossbar_bus_common::messages::Message;
use log::*;
use serde::{de::DeserializeOwned, Serialize};
use tokio::{
    net::UnixStream,
    sync::{
        broadcast::Receiver as BroadcastReceiver,
        mpsc::{self, Receiver, Sender},
    },
};
use tokio_stream::{Stream, StreamExt};

use krossbar_common_messages::{Error, Message as MessageBody, Response};
use krossbar_common_rpc::{
    rpc_connection::RpcConnection, rpc_sender::RpcSender, Message as MessageHandle,
};

use crate::monitor::Monitor;

use super::peer_connector::PeerConnector;

/// A command from outside into the loop
enum CommandType {
    Monitor(Monitor),
    Shutdown,
}

/// P2p service connection handle
#[derive(Clone)]
pub struct Peer {
    /// Own service name
    service_name: String,
    /// Peer service name
    peer_service_name: String,
    /// We reconnect only to the outgoing peer connections. This is passed to the connector.
    /// This value can change, because we can start subscribing to a service, which
    /// previously initiated connection.
    outgoing: Arc<AtomicBool>,
    /// Sender into the socket to return to the client
    peer_sender: RpcSender,
    /// Sender to shutdown peer connection or set monitor
    command_tx: Sender<CommandType>,
}

impl Peer {
    /// Create new service handle and start tokio task to handle incoming messages from the peer
    /// *incoming_stream* If passed, this is an incoming connection, if None, peer handle should
    ///     connect itself
    /// *hub_writer* Sends a message directrly to the hub. Used for reconnection
    pub(crate) async fn new(
        service_name: String,
        peer_service_name: String,
        incoming_stream: Option<UnixStream>,
        endpoints_tx: Sender<MessageHandle>,
        hub_writer: RpcSender,
    ) -> Result<Self> {
        // If we have an incoming stream, we don't reconnect
        let outgoing: Arc<AtomicBool> = Arc::new(if incoming_stream.is_none() {
            true.into()
        } else {
            false.into()
        });

        info!(
            "Registered new {} connection from {}",
            if incoming_stream.is_none() {
                "outgoing"
            } else {
                "incoming"
            },
            peer_service_name
        );

        let (command_tx, command_rx) = mpsc::channel(1);

        // Peer connector, which will connect to the peer if this is and outgoing connection
        let connector = Box::new(PeerConnector::new(
            peer_service_name.clone(),
            hub_writer,
            incoming_stream,
            outgoing.clone(),
        ));

        // Rpc connection
        let rpc_connection = RpcConnection::new(connector).await?;

        let peer_sender = rpc_connection.sender();

        Self::start_task(
            rpc_connection,
            command_rx,
            endpoints_tx,
            service_name.clone(),
            peer_service_name.clone(),
        );

        Ok(Self {
            service_name: service_name.clone(),
            peer_service_name: peer_service_name.clone(),
            outgoing,
            peer_sender,
            command_tx,
        })
    }

    async fn start_task(
        mut rpc_connection: RpcConnection,
        mut shutdown_rx: Receiver<CommandType>,
        endpoints_tx: Sender<MessageHandle>,
        service_name: String,
        peer_service_name: String,
    ) {
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    // Read incoming message from the peer. RpcConnection::read return only if we've got
                    // an incoming message, not a response.
                    // If disconnected, we receive Response::Shutdowm, and after that start
                    // sending None to exclude peer handle from polling. Also, this message is sent when a peer
                    // shutting down gracefully
                    incoming_message = rpc_connection.read() => {
                        match incoming_message {
                            Ok(message) => {
                                if matches!(message.body(), MessageBody::Response(Response::Shutdown(_))) {
                                    warn!("Peer connection closed. Shutting him down");
                                    return;
                                } else {
                                    // This is an incoming message. Send it to the interfaces
                                    if endpoints_tx.send(message).await.is_err() {
                                        warn!("Peer connection closed. Shutting down");
                                        return;
                                    }
                                }
                            },
                            Err(err) => {
                                warn!("Peer connection closed. Shutting down");
                                return;
                            }
                        }
                    },
                    Some(message) = shutdown_rx.recv() => {
                        match message {
                            CommandType::Monitor(monitor) => rpc_connection.set_monitor(Box::new(monitor)),
                            CommandType::Shutdown => {
                                let message: MessageBody = Response::Shutdown("Shutdown".into()).into();
                                rpc_connection.sender().send(message.into());
                                return;
                            }
                        }
                    }
                };
            }
        });
    }

    pub fn name(&self) -> &String {
        &self.service_name
    }

    /// Remote method call\
    /// **P** is an argument type. Should be a serializable structure.\
    /// **R** is return type. Should be a deserializable structure
    pub async fn call<P: Serialize, R: DeserializeOwned>(
        &mut self,
        method_name: &str,
        params: &P,
    ) -> Result<R> {
        let message =
            MessageBody::new_call(self.peer_service_name.clone(), method_name.into(), params);

        // Send method call request
        let response: MessageBody = self.peer_sender.call(&message).await?.body();

        match response {
            // Succesfully performed remote method call
            MessageBody::Response(Response::Return(data)) => {
                match bson::from_bson::<R>(data.clone()) {
                    Ok(data) => Ok(data),
                    Err(err) => {
                        error!("Can't deserialize method response: {}", err.to_string());
                        Err(Error::InvalidResponse.into())
                    }
                }
            }
            // Got an error from the peer
            MessageBody::Response(Response::Error(err)) => {
                warn!(
                    "Failed to perform a call to `{}::{}`: {}",
                    self.peer_service_name,
                    method_name,
                    err.to_string()
                );
                Err(err.into())
            }
            // Invalid protocol
            r => {
                error!("Invalid Ok response for a method call: {:?}", r);
                Err(Error::InvalidMessage.into())
            }
        }
    }

    /// Remote signal subscription\
    /// **T** is the signal type. Should be a deserializable structure
    pub async fn subscribe<T>(&mut self, signal_name: &str) -> Result<impl Stream<Item = T>>
    where
        T: DeserializeOwned + Send,
    {
        let message = MessageBody::new_subscription(self.service_name.clone(), signal_name.into());

        let mut subscription_stream = self.peer_sender.subscribe(&message).await?;

        let subscription_response: MessageBody = subscription_stream
            .next()
            .await
            .context("Subscription stream unexpectedly closed")?
            .body();

        // Match first message
        match subscription_response {
            // Succesfully performed remote method call
            MessageBody::Response(Response::Ok) => {
                debug!("Succesfully subscribed to the signal `{}`", signal_name);

                Ok(subscription_stream.map_while(|message| {
                    let bson_body = message.body::<Bson>();

                    if let Ok(body) = bson::from_bson(bson_body.clone()) {
                        Some(body)
                    } else {
                        warn!(
                            "Failed to deserialize subscription body from {:?}",
                            bson_body
                        );
                        None
                    }
                }))
            }
            // Invalid protocol
            r => {
                error!("Invalid Ok response for a signal subscription: {:?}", r);
                Err(Error::InvalidMessage.into())
            }
        }
    }

    /// Start watching remote state changes\
    /// **T** is the signal type. Should be a deserializable structure\
    /// **Returns** current state value
    pub async fn watch<T>(&mut self, state_name: &str) -> Result<impl Stream<Item = T>>
    where
        T: DeserializeOwned + Send,
    {
        let message = MessageBody::new_watch(self.service_name.clone(), state_name.into());

        let mut subscription_stream = self.peer_sender.subscribe(&message).await?;

        // Match first message
        match subscription_stream
            .next()
            .await
            .context("Watch stream unexpectedly closed")?
            .body()
        {
            // Succesfully performed remote method call
            MessageBody::Response(Response::Ok) => {
                debug!("Succesfully started watching the state `{}`", state_name);

                Ok(subscription_stream.map_while(|message| {
                    let bson_body = message.body::<Bson>();

                    if let Ok(body) = bson::from_bson(bson_body.clone()) {
                        Some(body)
                    } else {
                        warn!("Failed to deserialize watch body from {:?}", bson_body);
                        None
                    }
                }))
            }
            // Invalid protocol
            r => {
                error!("Invalid Ok response for a signal subscription: {:?}", r);
                Err(Error::InvalidMessage.into())
            }
        }
    }

    /// Start subscription task, which polls signal Receiver and sends peer message
    /// if emited
    pub(crate) fn start_signal_sending_task(
        &self,
        mut signal_receiver: BroadcastReceiver<Message>,
        seq: u64,
    ) {
        tokio::spawn(async move {
            loop {
                // Wait for signal emission
                match signal_receiver.recv().await {
                    Ok(mut message) => {
                        // Replace seq with subscription seq
                        message.update_seq(seq);

                        // Call self task to send signal message
                        if let Err(_) = self
                            .peer_sender
                            .send(bson::to_bson(&message).unwrap())
                            .await
                        {
                            warn!("Failed to send signal to a subscriber. Probably closed. Removing subscriber");
                            return;
                        }
                    }
                    Err(err) => {
                        error!("Signal receiver error: {:?}", err);
                        return;
                    }
                }
            }
        });
    }

    pub async fn set_monitor(&mut self, monitor: &Monitor) {
        debug!(
            "Incoming monitor connection for the peer {}",
            self.peer_service_name
        );

        let new_mon_instance = monitor.clone_for_service(&self.peer_service_name.clone());
        self.command_tx
            .send(CommandType::Monitor(new_mon_instance))
            .await;
    }

    /// Close peer connection
    pub async fn close(&mut self) {
        let self_name = self.peer_service_name.clone();
        debug!(
            "Shutting down peer connection to `{}`",
            self.peer_service_name
        );

        self.command_tx.send(CommandType::Shutdown).await;
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        trace!("Peer `{}` connection dropped", self.peer_service_name);

        let shutdown_tx = self.command_tx.clone();

        tokio::spawn(async move {
            let _ = shutdown_tx.send(CommandType::Shutdown).await;
        });
    }
}

impl Debug for Peer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Peer connection to {}", self.peer_service_name)
    }
}
