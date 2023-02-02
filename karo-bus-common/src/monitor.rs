use bson::Bson;
use karo_common_messages::Message;
use serde::{Deserialize, Serialize};

pub const MONITOR_SERVICE_NAME: &str = "karo.bus.monitor";
pub const MONITOR_METHOD: &str = "message";

/// Monitor message directions. Used for more convenient formatting
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum MonitorMessageDirection {
    Incoming,
    Outgoing,
}

/// Monitor receiving method argument type
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
            message: message.into(),
            direction,
        }
    }
}
