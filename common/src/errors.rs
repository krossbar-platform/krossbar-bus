use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Serialize, Deserialize, Debug, Error)]
pub enum Error {
    #[error("Connection with service now allowed")]
    NotAllowed,
    #[error("Service not found")]
    NotFound,
    #[error("Service with this name is already registered")]
    NameRegistered,
    #[error("Invalid protocol version. Please update Caro dependencies")]
    InvalidProtocol,
}
