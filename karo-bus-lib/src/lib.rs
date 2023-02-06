pub mod bus;
mod hub_connection;
mod monitor;
pub mod peer;
mod peer_connector;
pub mod signal;
#[cfg(feature = "simple-peer")]
pub mod simple_peer;
pub mod state;
mod utils;

pub use bus::Bus;

pub type Error = Box<dyn std::error::Error + Sync + Send>;
pub type Result<T> = std::result::Result<T, Error>;
