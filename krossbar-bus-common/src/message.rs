use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub enum HubMessage {
    Register { service_name: String },
    Connect { service_name: String, wait: bool },
}
