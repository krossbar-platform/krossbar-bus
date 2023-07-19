use std::{
    any::type_name,
    collections::HashMap,
    future::Future,
    sync::{Arc, RwLock},
};

use anyhow::Result;
use bson::Bson;
use log::*;
use serde::{de::DeserializeOwned, Serialize};

use tokio::sync::{
    broadcast::{self, Sender as BroadcastSender},
    mpsc::{self, Receiver, Sender},
    oneshot::{self, Sender as OneSender},
    watch::{self, Receiver as WatchReceiver},
};

use karo_bus_common::{
    errors::Error as BusError,
    inspect_data::InspectData,
    messages::{IntoMessage, Message, Response},
};

use karo_common_rpc::Message as MessageHandle;

use crate::{
    connections::peer::Peer,
    endpoints::{signal::Signal, state::State},
};

pub mod signal;
pub mod state;

type Shared<T> = Arc<RwLock<T>>;
type MethodCall = (Bson, OneSender<Response>);

/// This service endpoints
#[derive(Clone)]
pub struct Endpoints {
    /// Registered methods. Sender is used to send parameters and
    /// receive a result from a callback
    methods: Shared<HashMap<String, Sender<MethodCall>>>,
    /// Registered signals. Sender is used to emit signals to subscribers
    signals: Shared<HashMap<String, BroadcastSender<Message>>>,
    /// Registered states.
    /// Sender is used to nofity state change to subscribers
    /// Receiver used to get current walue when user makes watch request
    states: Shared<HashMap<String, (BroadcastSender<Message>, WatchReceiver<Bson>)>>,
    /// Data for service inspection
    inspect_data: Shared<InspectData>,
}

impl Endpoints {
    pub fn new() -> Self {
        Self {
            methods: Arc::new(RwLock::new(HashMap::new())),
            signals: Arc::new(RwLock::new(HashMap::new())),
            states: Arc::new(RwLock::new(HashMap::new())),
            inspect_data: Arc::new(RwLock::new(InspectData::new())),
        }
    }
    /// Register service method. The function uses BSON internally for requests
    /// and responses.\
    /// **P** is paramtere type. Should be a deserializable structure\
    /// **R** is method return type. Should be a serializable structure
    pub fn register_method<P, R, Ret>(
        &mut self,
        method_name: &str,
        callback: impl Fn(P) -> Ret + Send + Sync + 'static,
    ) -> Result<()>
    where
        P: DeserializeOwned + Send + 'static,
        R: Serialize + Send + 'static,
        Ret: Future<Output = R> + Send,
    {
        let method_name = method_name.into();
        let mut rx = self.update_method_map(&method_name)?;

        // Add the method into the inspection register
        self.inspect_data.write().unwrap().methods.push(format!(
            "{}({}) -> {}",
            method_name,
            type_name::<P>().split("::").last().unwrap_or("Unknown"),
            type_name::<R>().split("::").last().unwrap_or("Unknown")
        ));

        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Some((params, calback_tx)) => {
                        match bson::from_bson::<P>(params) {
                            Ok(params) => {
                                // Receive method call response
                                let result = callback(params).await;

                                // Deserialize and send user response
                                calback_tx
                                    .send(Response::Return(bson::to_bson(&result).unwrap()))
                                    .unwrap();
                            }
                            Err(err) => {
                                warn!(
                                    "Failed to deserialize method call parameters: {}",
                                    err.to_string()
                                );

                                calback_tx
                                    .send(Response::Error(BusError::InvalidParameters(
                                        err.to_string(),
                                    )))
                                    .unwrap();
                            }
                        }
                    }
                    None => {
                        trace!("Method {} shut down", method_name);
                        return;
                    }
                }
            }
        });

        Ok(())
    }

    /// Adds new method to a method map
    fn update_method_map(&mut self, method_name: &String) -> Result<Receiver<MethodCall>> {
        // The function just creates a method handle, which performs type conversions
        // for incoming data and client replies. See [Method] for details
        let mut methods = self.methods.write().unwrap();

        if methods.contains_key(method_name) {
            error!(
                "Failed to register method `{}`. Already registered",
                method_name
            );

            return Err(BusError::AlreadyRegistered.into());
        }

        let (tx, rx) = mpsc::channel(32);

        methods.insert(method_name.clone(), tx);

        info!("Succesfully registered method: {}", method_name);
        Ok(rx)
    }

    /// Register service signal.\
    /// **T** is a signal type. Should be a serializable structure.\
    /// **Returns** [Signal] handle which can be used to emit signal
    pub fn register_signal<T>(&mut self, signal_name: &str) -> Result<Signal<T>>
    where
        T: Serialize + 'static,
    {
        let mut signals = self.signals.write().unwrap();

        if signals.contains_key(signal_name) {
            error!(
                "Failed to register signal `{}`. Already registered",
                signal_name
            );

            return Err(BusError::AlreadyRegistered.into());
        }

        // Add the signal into the inspection register
        self.inspect_data.write().unwrap().signals.push(format!(
            "{}: {}",
            signal_name,
            type_name::<T>().split("::").last().unwrap_or("Unknown"),
        ));

        let (tx, _rx) = broadcast::channel(5);

        signals.insert(signal_name.into(), tx.clone());

        info!("Succesfully registered signal: {}", signal_name);
        Ok(Signal::new(signal_name.into(), tx))
    }

    /// Register service signal.\
    /// **T** is a signal type. Should be a serializable structure.\
    /// **Returns** [State] handle which can be used to change state. Settings the state
    /// will emit state change to watchers.
    pub fn register_state<T>(&mut self, state_name: &str, initial_value: T) -> Result<State<T>>
    where
        T: Serialize + 'static,
    {
        let mut states = self.states.write().unwrap();

        if states.contains_key(state_name) {
            error!(
                "Failed to register state `{}`. Already registered",
                state_name
            );

            return Err(BusError::AlreadyRegistered.into());
        }

        // Add the state into the inspection register
        self.inspect_data.write().unwrap().states.push(format!(
            "{}: {}",
            state_name,
            type_name::<T>().split("::").last().unwrap_or("Unknown"),
        ));

        // Channel to send state update to subscribers
        let (tx, _rx) = broadcast::channel(5);

        // Channel to get current value when someone is subscribing
        let bson = bson::to_bson(&initial_value).unwrap();
        let (watch_tx, watch_rx) = watch::channel(bson);

        states.insert(state_name.into(), (tx.clone(), watch_rx));

        info!("Succesfully registered state: {}", state_name);
        Ok(State::new(state_name.into(), initial_value, tx, watch_tx))
    }

    /// Handle incoming method call
    pub async fn handle_method_call(
        &self,
        caller_name: &str,
        method_name: &str,
        params: &Bson,
        handle: &mut MessageHandle,
    ) {
        debug!(
            "Service `{}` requested method `{}` call",
            caller_name, method_name
        );

        let seq = handle.id();

        if method_name == karo_bus_common::inspect_data::INSPECT_METHOD {
            handle.reply(&self.handle_inspect_call(seq)).await;
            return;
        }

        let method = self.methods.read().unwrap().get(method_name).cloned();

        let response = if let Some(method) = method {
            // Create oneshot channel to receive response
            let (tx, rx) = oneshot::channel();

            // Call user
            method.send((params.clone(), tx)).await.unwrap();
            // Await for his respons
            rx.await.unwrap().into_message(seq)
        } else {
            BusError::NotRegistered.into_message(seq)
        };

        handle.reply(&response);
    }

    /// Handle incoming method call
    pub fn handle_inspect_call(&self, seq: u64) -> Message {
        Response::Return(bson::to_bson(&*self.inspect_data.read().unwrap()).unwrap())
            .into_message(seq)
    }

    /// Handle incoming signal subscription
    pub async fn handle_incoming_signal_subscription(
        &self,
        subscriber_name: &str,
        signal_name: &str,
        handle: &mut MessageHandle,
        peer: &Peer,
    ) {
        debug!(
            "Service `{}` requested signal `{}` subscription",
            subscriber_name, signal_name
        );

        let seq = handle.id();
        let signal = self.signals.read().unwrap().get(signal_name).cloned();

        let response = if let Some(signal_sender) = signal {
            peer.start_signal_sending_task(signal_sender.subscribe(), seq);
            Response::Ok.into_message(seq)
        } else {
            BusError::NotRegistered.into_message(seq)
        };

        handle.reply(&response);
    }

    /// Handle incoming request to watch state
    pub async fn handle_incoming_state_watch(
        &self,
        subscriber_name: &str,
        state_name: &str,
        handle: &mut MessageHandle,
        peer: &Peer,
    ) {
        debug!(
            "Service `{}` requested state `{}` watch",
            subscriber_name, state_name
        );

        let seq = handle.id();
        let state = self.states.read().unwrap().get(state_name).cloned();

        let response = if let Some((state_change_sender, value_watch)) = state {
            let current_value = value_watch.borrow().clone();

            peer.start_signal_sending_task(state_change_sender.subscribe(), seq);
            Response::StateChanged(current_value).into_message(seq)
        } else {
            BusError::NotRegistered.into_message(seq)
        };

        handle.reply(&response);
    }
}
