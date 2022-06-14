use super::messages::{Message, Response};
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
    #[error("Method already registered")]
    MethodCallError(String),
    #[error("Method not registered")]
    MethodRegistered,
    #[error("Invalid protocol version. Please update Caro dependencies")]
    InvalidProtocol,
    #[error("Invalid parameters passed into a function")]
    InvalidParameters,
    #[error("Invalid return type from a function. Can't deserialize response")]
    InvalidResponse,
    #[error("Internal bus error. See logs for details")]
    Internal,
}

impl Into<Message> for Error {
    fn into(self) -> Message {
        Message::Response(Response::Error(self))
    }
}
