use std::{
    error::Error,
    fmt::Debug,
    future::Future,
    sync::{atomic::AtomicBool, Arc},
};

use karo_common_rpc::rpc_sender::RpcSender;
use log::*;
use serde::{de::DeserializeOwned, Serialize};
use tokio::{
    net::UnixStream,
    sync::{
        broadcast::Receiver as BroadcastReceiver,
        mpsc::{self, Receiver, Sender},
        RwLock as TokioRwLock,
    },
};

use crate::utils::{self, dummy_tx, TaskChannel};
use karo_bus_common::{
    errors::Error as BusError,
    messages::{self, IntoMessage, Message, MessageBody, Response},
};

/// P2p service connection handle
#[derive(Clone)]
pub struct Peer {
    /// Own service name
    service_name: String,
    /// Peer service name
    peer_service_name: String,
    /// Sender to forward calls to the service
    service_tx: TaskChannel,
    /// Peer sender, which can be given to a client to send message to the peer
    sender: RpcSender,
    /// We reconnect only to the outgoing peer connections. This is passed to the connector.
    /// This value can change, because we can start subscribing to a service, which
    /// previously initiated connection.
    outgoing: Arc<AtomicBool>,
    /// Sender to shutdown peer connection
    shutdown_tx: Sender<()>,
    /// Monitor connection
    monitor: Option<RpcSender>,
}

impl Peer {
    /// Create new service handle and start tokio task to handle incoming messages from the peer
    pub(crate) fn new(
        service_name: String,
        peer_service_name: String,
        stream: UnixStream,
        service_tx: TaskChannel,
        outgoing: bool,
    ) -> Self {
        info!(
            "Registered new {} connection from {}",
            if outgoing { "outgoing" } else { "incoming" },
            peer_service_name
        );

        let (task_tx, mut task_rx) = mpsc::channel(32);
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);

        let mut this = Self {
            service_name: service_name.clone(),
            peer_service_name: peer_service_name.clone(),
            service_tx: service_tx.clone(),
            task_tx,
            shutdown_tx,
        };
        let result = this.clone();

        tokio::spawn(async move {
            let mut peer_handle = PeerConnection::new(
                peer_service_name,
                stream,
                service_tx,
                service_name.clone(),
                outgoing,
            );

            loop {
                tokio::select! {
                    // Read incoming message from the peer. This one is tricky: if peer is alive, we always
                    // send Some(message). If disconnected, we send Response::Shutdowm, and after that start
                    // sending None to exclude peer handle from polling. Also, this message is sent when a peer
                    // shutting down gracefully
                    Some(message) = peer_handle.read_message() => {
                        if matches!(message.body(), MessageBody::Response(Response::Shutdown(_))) {
                            warn!("Peer connection closed. Shutting him down");
                            this.close().await
                        } else {
                            // Peer handle resolves call itself. If message returned, redirect to
                            // the service connection
                            let response = this.handle_peer_message(message).await;

                            if peer_handle.write_message(response, dummy_tx()).await.is_none() {
                                warn!("Peer connection closed. Shutting down");
                                this.close().await
                            }
                        }
                    },
                    // Handle method calls
                    Some((request, callback_tx)) = task_rx.recv() => {
                        match request {
                            TaskMessage::Message(message) => {
                                trace!("Peer task message: {:?}", message);

                                if peer_handle.write_message(message, callback_tx).await.is_none() {
                                    warn!("Peer connection closed. Shutting down");
                                    this.close().await
                                }
                            },
                            TaskMessage::Monitor(monitor) => peer_handle.set_monitor(monitor),
                        }

                    },
                    Some(_) = shutdown_rx.recv() => {
                        let shutdown_message = Response::Shutdown("Shutdown".into()).into_message(0);
                        let _ = peer_handle.write_message(shutdown_message, dummy_tx()).await;
                        peer_handle.shutdown().await;
                        drop(peer_handle);
                        return
                    }
                };
            }
        });

        result
    }

    pub fn name(&self) -> &String {
        &self.service_name
    }

    /// Remote method call\
    /// **P** is an argument type. Should be a serializable structure.\
    /// **R** is return type. Should be a deserializable structure
    pub async fn call<P: Serialize, R: DeserializeOwned>(
        &self,
        method_name: &str,
        params: &P,
    ) -> crate::Result<R> {
        let message = Message::new_call(self.peer_service_name.clone(), method_name.into(), params);

        // Send method call request
        let response = utils::call_task(&self.task_tx, TaskMessage::Message(message)).await;

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
                        self.peer_service_name,
                        method_name,
                        err.to_string()
                    );
                    Err(Box::new(err.clone()))
                }
                // Invalid protocol
                r => {
                    error!("Invalid Ok response for a method call: {:?}", r);
                    Err(Box::new(BusError::InvalidMessage))
                }
            },
            // Network error
            Err(e) => {
                error!("Ivalid error response from a method call: {:?}", e);
                Err(e)
            }
        }
    }

    /// Remote signal subscription\
    /// **T** is the signal type. Should be a deserializable structure
    pub async fn subscribe<T, Ret>(
        &self,
        signal_name: &str,
        callback: impl Fn(T) -> Ret + Send + Sync + 'static,
    ) -> crate::Result<()>
    where
        T: DeserializeOwned + Send,
        Ret: Future<Output = ()> + Send,
    {
        let message = Message::new_subscription(self.service_name.clone(), signal_name.into());

        let (response, rx) = self.make_subscription_call(message, signal_name).await?;

        match response.body() {
            // Succesfully performed remote method call
            MessageBody::Response(Response::Ok) => {
                debug!("Succesfully subscribed to the signal `{}`", signal_name);

                Peer::start_subscription_receiving_task(signal_name, rx, callback);

                Ok(())
            }
            // Invalid protocol
            r => {
                error!("Invalid Ok response for a signal subscription: {:?}", r);
                Err(Box::new(BusError::InvalidMessage))
            }
        }
    }

    /// Start watching remote state changes\
    /// **T** is the signal type. Should be a deserializable structure\
    /// **Returns** current state value
    pub async fn watch<T, Ret>(
        &self,
        state_name: &str,
        callback: impl Fn(T) -> Ret + Send + Sync + 'static,
    ) -> crate::Result<T>
    where
        T: DeserializeOwned + Send,
        Ret: Future<Output = ()> + Send,
    {
        let message = Message::new_watch(self.service_name.clone(), state_name.into());

        let (response, rx) = self.make_subscription_call(message, state_name).await?;

        match response.body() {
            // Succesfully performed remote method call
            MessageBody::Response(Response::StateChanged(data)) => {
                let state = match bson::from_bson::<T>(data.clone()) {
                    Ok(data) => Ok(data),
                    Err(err) => {
                        error!("Can't deserialize state response: {}", err.to_string());
                        Err(Box::new(BusError::InvalidResponse))
                    }
                }?;

                debug!("Succesfully started watching state `{}`", state_name);

                Peer::start_subscription_receiving_task(state_name, rx, callback);

                Ok(state)
            }
            // Invalid protocol
            r => {
                error!("Invalid Ok response for a signal subscription: {:?}", r);
                Err(Box::new(BusError::InvalidMessage))
            }
        }
    }

    /// Make subscription call and get result
    /// Used for both: signals and states
    async fn make_subscription_call(
        &self,
        message: Message,
        signal_name: &str,
    ) -> Result<(Message, Receiver<Message>), Box<dyn Error + Sync + Send>> {
        // This is a tricky one. First we use channel to read subscription status, and after that
        // for incomin signal emission
        let (tx, mut rx) = mpsc::channel(10);

        // Send subscription request
        self.task_tx
            .send((TaskMessage::Message(message), tx))
            .await?;

        let response = rx.recv().await.unwrap();

        match response.body() {
            // Got an error from the peer
            MessageBody::Response(Response::Error(err)) => {
                warn!(
                    "Failed to subscribe to `{}::{}`: {}",
                    self.peer_service_name,
                    signal_name,
                    err.to_string()
                );
                Err(Box::new(err.clone()))
            }
            // Invalid protocol
            _ => Ok((response, rx)),
        }
    }

    /// Start task to receive signals emission, state changes and calling user callback
    fn start_subscription_receiving_task<T, Ret>(
        signal_name: &str,
        mut receiver: Receiver<Message>,
        callback: impl Fn(T) -> Ret + Send + Sync + 'static,
    ) where
        T: DeserializeOwned + Send,
        Ret: Future<Output = ()> + Send,
    {
        let signal_name: String = signal_name.into();

        // Start listening to signal emissions
        tokio::spawn(async move {
            loop {
                match receiver.recv().await {
                    Some(message) => {
                        match message.body() {
                            // Signal
                            MessageBody::Response(Response::Signal(value)) => {
                                match bson::from_bson::<T>(value.clone()) {
                                    Ok(value) => {
                                        trace!("Signal response");
                                        // Call back
                                        callback(value).await;
                                    }
                                    Err(err) => {
                                        error!(
                                            "Failed to deserialize signal value: {}",
                                            err.to_string()
                                        );
                                    }
                                }
                            }
                            MessageBody::Response(Response::StateChanged(value)) => {
                                match bson::from_bson::<T>(value.clone()) {
                                    Ok(value) => {
                                        trace!("State change");
                                        // Call back
                                        callback(value).await;
                                    }
                                    Err(err) => {
                                        error!(
                                            "Failed to deserialize state value: {}",
                                            err.to_string()
                                        );
                                    }
                                }
                            }
                            // Subscriptions response Ok
                            MessageBody::Response(Response::Ok) => {}
                            m => {
                                error!("Invalid message inside signal handling code: {:?}", m);
                            }
                        }
                    }
                    None => {
                        error!(
                            "Failed to listen to signal `{}` subscription. Cancelling",
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
    pub(crate) fn start_signal_sending_task(
        &self,
        mut signal_receiver: BroadcastReceiver<Message>,
        seq: u64,
    ) {
        let self_tx = self.task_tx.clone();

        tokio::spawn(async move {
            loop {
                // Wait for signal emission
                match signal_receiver.recv().await {
                    Ok(mut message) => {
                        // Replace seq with subscription seq
                        message.update_seq(seq);

                        // Call self task to send signal message
                        if let Err(_) =
                            utils::call_task(&self_tx, TaskMessage::Message(message)).await
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

                BusError::Internal.into_message(response_seq)
            }
        }
    }

    /// Set monitor handle
    pub(crate) async fn set_monitor(
        &mut self,
        monitor_connection: Arc<TokioRwLock<PeerConnection>>,
    ) {
        let _ = self
            .task_tx
            .send((TaskMessage::Monitor(monitor_connection), dummy_tx()))
            .await;
    }

    /// Send message to the Karo monitor if connected
    async fn send_monitor_message(
        &mut self,
        message: &Message,
        direction: MonitorMessageDirection,
    ) {
        let (sender, receiver) = match direction {
            MonitorMessageDirection::Outgoing => (self.service_name.clone(), self.name.clone()),
            MonitorMessageDirection::Incoming => (self.name.clone(), self.service_name.clone()),
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

    /// Close peer connection
    pub async fn close(&mut self) {
        let self_name = self.peer_service_name.clone();
        debug!(
            "Shutting down peer connection to `{}`",
            self.peer_service_name
        );

        let _ = utils::call_task(
            &self.service_tx,
            Response::Shutdown(self_name.clone()).into_message(0),
        );
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        trace!("Peer `{}` connection dropped", self.peer_service_name);

        let shutdown_tx = self.shutdown_tx.clone();

        tokio::spawn(async move {
            let _ = shutdown_tx.send(()).await;
        });
    }
}

impl Debug for Peer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Peer connection to {}", self.peer_service_name)
    }
}
