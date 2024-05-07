use super::messages::{IntoMessage, Message, MessageBody, Response};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Serialize, Deserialize, Debug, Error, Clone)]
pub enum Error {
    #[error("Not allowed")]
    NotAllowed,
    #[error("Service not found")]
    ServiceNotFound,
    #[error("Service not registered yet")]
    ServiceNotRegisterd,
    #[error("Service with this name is already registered")]
    NameRegistered,
    #[error("Method, Signal, or State with a given name already registered")]
    AlreadyRegistered,
    #[error("Service, Method, Signal, or State not registered")]
    NotRegistered,
    #[error("Invalid protocol version. Please update Krossbar dependencies")]
    InvalidProtocol,
    #[error("Invalid parameters passed into a function {0}")]
    InvalidParameters(String),
    #[error("Invalid return type from a function. Can't deserialize response")]
    InvalidResponse,
    #[error("Invalid message received. Internal error. Please fill bug report")]
    InvalidMessage,
    #[error("Service not connected")]
    NotConnected,
    #[error("Internal bus error. See logs for details. Please fill bug report")]
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
