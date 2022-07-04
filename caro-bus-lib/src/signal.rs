use std::marker::PhantomData;

use log::error;
use serde::Serialize;
use tokio::sync::broadcast::Sender as BroadcastSender;

use caro_bus_common::messages::{IntoMessage, Message, Response};

/// Signal handle, which can be used for signal emission
pub struct Signal<T: Serialize> {
    /// Sender used by subscribers to reseive emissions
    tx: BroadcastSender<Message>,
    /// Registered signal name
    name: String,
    _phantom: PhantomData<T>,
}

impl<T: Serialize> Signal<T> {
    pub(crate) fn new(name: String, tx: BroadcastSender<Message>) -> Self {
        Self {
            tx,
            name,
            _phantom: PhantomData,
        }
    }

    /// Emit signal with value of type **T**
    pub fn emit(&self, value: T) {
        if self.tx.receiver_count() == 0 {
            return;
        }

        let message = Response::Signal(bson::to_bson(&value).unwrap()).into_message(0xFEEDC0DE);

        if let Err(err) = self.tx.send(message) {
            error!("Failed to emit signal `{}`: {:?}", self.name, err);
        }
    }
}
