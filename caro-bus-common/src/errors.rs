use super::messages::{IntoMessage, Message, MessageBody, Response};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Serialize, Deserialize, Debug, Error, Clone)]
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
    MethodNotRegistered,
    #[error("Invalid protocol version. Please update Caro dependencies")]
    InvalidProtocol,
    #[error("Invalid parameters passed into a function")]
    InvalidParameters,
    #[error("Invalid return type from a function. Can't deserialize response")]
    InvalidResponse,
    #[error("Internal bus error. See logs for details")]
    Internal,
}

impl IntoMessage for Error {
    fn into_message(self, seq: u64) -> Message {
        Message {
            seq,
            body: MessageBody::Response(Response::Error(self)),
        }
    }
}
