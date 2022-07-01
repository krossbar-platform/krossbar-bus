use bson::Bson;
use serde::{Deserialize, Serialize};

use crate::messages::Message;

pub const MONITOR_SERVICE_NAME: &str = "com.bus.monitor";
pub const MONITOR_METHOD: &str = "message";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum MonitorMessageDirection {
    Incoming,
    Outgoing,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MonitorMessage {
    pub sender: String,
    pub receiver: String,
    /// This should be a serialized [Message]
    pub message: Bson,
    pub direction: MonitorMessageDirection,
}

impl MonitorMessage {
    pub fn new(
        sender: String,
        receiver: String,
        message: &Message,
        direction: MonitorMessageDirection,
    ) -> Self {
        Self {
            sender,
            receiver,
            message: bson::to_bson(message).unwrap(),
            direction,
        }
    }
}