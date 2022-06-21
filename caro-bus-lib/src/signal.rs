use std::marker::PhantomData;

use log::error;
use serde::Serialize;
use tokio::sync::watch::Sender as WatchSender;

use caro_bus_common::messages::{IntoMessage, Message, Response};

pub struct Signal<T: Serialize> {
    tx: WatchSender<Message>,
    name: String,
    _phantom: PhantomData<T>,
}

impl<T: Serialize> Signal<T> {
    pub fn new(name: String, tx: WatchSender<Message>) -> Self {
        Self {
            tx,
            name,
            _phantom: PhantomData,
        }
    }
    pub fn emit(&self, value: T) {
        let message = Response::Return(bson::to_bson(&value).unwrap()).into_message(0xFEEDC0DE);

        if let Err(err) = self.tx.send(message) {
            error!("Failed to emit signal {}: {:?}", self.name, err);
        }
    }
}
