use bson::Bson;
use log::{error, warn};
use serde::Serialize;
use tokio::sync::{broadcast::Sender as BroadcastSender, watch::Sender as WatchSender};

use caro_bus_common::messages::{IntoMessage, Message, Response};

pub type ExternalStateGetter = Box<dyn Fn() -> Bson + Send + Sync>;

pub struct State<T: Serialize> {
    tx: BroadcastSender<Message>,
    watch_tx: WatchSender<Bson>,
    name: String,
    value: T,
}

impl<T: Serialize> State<T> {
    pub fn new(
        name: String,
        value: T,
        tx: BroadcastSender<Message>,
        watch_tx: WatchSender<Bson>,
    ) -> Self {
        Self {
            tx,
            watch_tx,
            name,
            value,
        }
    }

    pub fn set(&mut self, value: T) {
        if self.tx.receiver_count() == 0 {
            return;
        }

        let bson = bson::to_bson(&value).unwrap();
        self.value = value;

        // First notify watch so new clients could get current value
        if let Err(err) = self.watch_tx.send(bson.clone()) {
            warn!(
                "Failed to set watch value for a state `{}`: {:?}",
                self.name, err
            );
        }

        let message = Response::StateChanged(bson).into_message(0xFEEDC0DE);

        if let Err(err) = self.tx.send(message) {
            error!("Failed to send state schange `{}`: {:?}", self.name, err);
        }
    }

    pub fn get(&self) -> &T {
        &self.value
    }
}
