use serde::{Deserialize, Serialize};

pub const HUB_CONNECT_METHOD: &str = "connect";
pub const HUB_REGISTER_METHOD: &str = "register";

#[derive(Serialize, Deserialize, Debug)]
pub enum Message {
    Register { service_name: String },
    Connect { service_name: String, wait: bool },
}
