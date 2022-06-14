pub mod bus_connection;
mod method;
pub mod service_connection;
mod utils;

pub use bus_connection::BusConnection;
pub use caro_bus_common::errors::Error;

use std::error::Error as StdError;

use caro_bus_common::messages::Message;

pub type CallbackType = Result<Message, Box<dyn StdError + Send + Sync>>;
