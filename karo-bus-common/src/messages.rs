use std::{fmt::Display, os::unix::prelude::RawFd};

use bson::{self, Bson};
use bytes::{Buf, BytesMut};
use log::*;
use serde::{Deserialize, Serialize};

use super::errors;

pub const PROTOCOL_VERSION: i64 = 1;
const INVALID_SEQ: u64 = 0xDEADBEEF;

pub trait IntoMessage {
    fn into_message(self, seq: u64) -> Message;
}

/// Message sent over a wire
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Message {
    pub(crate) seq: u64,
    pub(crate) body: MessageBody,
}

/// Message body, which constains sent data
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum MessageBody {
    ServiceMessage(ServiceMessage),
    Response(Response),
    MethodCall {
        caller_name: String,
        method_name: String,
        params: Bson,
    },
    SignalSubscription {
        subscriber_name: String,
        signal_name: String,
    },
    StateSubscription {
        subscriber_name: String,
        state_name: String,
    },
}

impl IntoMessage for MessageBody {
    fn into_message(self, seq: u64) -> Message {
        Message { seq, body: self }
    }
}

impl Display for MessageBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ServiceMessage(service_message) => write!(f, "{}", service_message),
            Self::Response(response) => write!(f, "{}", response),
            Self::MethodCall {
                caller_name: _,
                method_name,
                params,
            } => write!(
                f,
                "Method '{}' call. Argument value: {}",
                method_name, params
            ),
            Self::SignalSubscription {
                subscriber_name: _,
                signal_name,
            } => write!(f, "Signal '{}' subscription request", signal_name),
            Self::StateSubscription {
                subscriber_name: _,
                state_name,
            } => write!(f, "State '{}' watch request", state_name),
        }
    }
}

impl Message {
    pub fn body(&self) -> &MessageBody {
        &self.body
    }

    pub fn seq(&self) -> u64 {
        self.seq
    }

    pub fn update_seq(&mut self, seq: u64) {
        self.seq = seq;
    }

    pub fn bytes(&self) -> Vec<u8> {
        bson::to_raw_document_buf(&self).unwrap().into_bytes()
    }

    pub fn new_registration(service_name: String) -> Self {
        Self {
            seq: INVALID_SEQ,
            body: MessageBody::ServiceMessage(ServiceMessage::Register {
                protocol_version: PROTOCOL_VERSION,
                service_name,
            }),
        }
    }

    pub fn new_connection(peer_service_name: String, await_connection: bool) -> Self {
        Self {
            seq: INVALID_SEQ,
            body: MessageBody::ServiceMessage(ServiceMessage::Connect {
                peer_service_name,
                await_connection,
            }),
        }
    }

    pub fn new_call<T: Serialize>(caller_name: String, method_name: String, data: &T) -> Self {
        Self {
            seq: INVALID_SEQ,
            body: MessageBody::MethodCall {
                caller_name,
                method_name,
                params: bson::to_bson(&data).unwrap(),
            },
        }
    }

    pub fn new_subscription(subscriber_name: String, signal_name: String) -> Self {
        Self {
            seq: INVALID_SEQ,
            body: MessageBody::SignalSubscription {
                subscriber_name,
                signal_name,
            },
        }
    }

    pub fn new_watch(subscriber_name: String, state_name: String) -> Self {
        Self {
            seq: INVALID_SEQ,
            body: MessageBody::StateSubscription {
                subscriber_name,
                state_name,
            },
        }
    }
}

/// Reading function can use this enum to notify if it read an entire message,
/// or need more data to read in order to deserialize incoming message
pub enum EitherMessage {
    FullMessage(Message),
    NeedMoreData(usize),
}

/// Internal service message
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ServiceMessage {
    /// Client sends message to Hub to resister with *service_name*
    Register {
        protocol_version: i64,
        service_name: String,
    },
    /// Client sends message to Hub to connect to *peer_service_name*
    /// If **await_connection** hub will be waiting for service to start
    Connect {
        peer_service_name: String,
        await_connection: bool,
    },
    /// If one Client performs connection, other Client receives this message to make
    /// p2p connection. Right after the messge we need to red peer UDS file descriptor
    IncomingPeerFd { peer_service_name: String },
    /// This one is internal message to return incoming FD to a caller
    PeerFd(RawFd),
}

impl IntoMessage for ServiceMessage {
    fn into_message(self, seq: u64) -> Message {
        Message {
            seq,
            body: MessageBody::ServiceMessage(self),
        }
    }
}

impl Display for ServiceMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Register {
                protocol_version,
                service_name,
            } => write!(
                f,
                "Registration request. Protocol version {}. Requested service name: '{}'",
                protocol_version, service_name
            ),
            Self::Connect {
                peer_service_name,
                await_connection,
            } => write!(
                f,
                "Connection request to '{}'. Will await?: {}",
                peer_service_name, await_connection
            ),
            Self::IncomingPeerFd { peer_service_name } => {
                write!(f, "Incoming FD for a peer '{}'", peer_service_name)
            }
            Self::PeerFd(_) => panic!("Should never be accessible outside of the lib"),
        }
    }
}

/// Call reponse. Including state changes and signal emissions
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Response {
    Ok,
    Shutdown(String),
    Return(Bson),
    Signal(Bson),
    StateChanged(Bson),
    Error(errors::Error),
}

impl IntoMessage for Response {
    fn into_message(self, seq: u64) -> Message {
        Message {
            seq,
            body: MessageBody::Response(self),
        }
    }
}

impl Display for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ok => write!(f, "Ok"),
            Self::Shutdown(reason) => write!(f, "Shutdown message. Reason: {}", reason),
            Self::Return(bson) => write!(f, "Method response: {}", bson),
            Self::Signal(bson) => write!(f, "Signal emitted: {}", bson),
            Self::StateChanged(bson) => write!(f, "State changed: {}", bson),
            Self::Error(err) => write!(f, "Error: {}", err),
        }
    }
}

/// Try to parse a Message from the buffer
/// *Returns*:
/// 1. EitherMessage::FullMessage(message) if completely read the message
/// 2. EitherMessage::NeedMoreData(len) if still need to read n bytes of data to get a message
/// **log** param if we need to log in the function
pub(crate) fn parse_buffer(buffer: &mut BytesMut, log: bool) -> EitherMessage {
    // Not enough data
    if buffer.len() < 4 {
        return EitherMessage::NeedMoreData(4);
    }

    match buffer.as_ref()[0..4].try_into() {
        Ok(frame_len_bytes) => {
            // First lets find BSON document len
            let frame_len = i32::from_le_bytes(frame_len_bytes) as usize;

            if log {
                trace!("Next frame len: {}. Buffer len {}", frame_len, buffer.len());
            }

            // If buffer contains complete document, try to parse
            if buffer.len() >= frame_len {
                if let Ok(frame) = bson::from_slice(buffer.as_ref()) {
                    buffer.advance(frame_len);

                    if log {
                        trace!("Incoming BSON message of len {}: {:?}", frame_len, frame);
                    }

                    // Full message read
                    return EitherMessage::FullMessage(frame);
                } else {
                    // TODO: Recover
                }
            } else {
                // Not enough data for a frame (Bson). Ask to read the rest of the message
                return EitherMessage::NeedMoreData(frame_len - buffer.len());
            }
        }
        Err(err) => {
            if log {
                error!("Failed to convert buffer in a slice: {}", err);
            }
        }
    }

    return EitherMessage::NeedMoreData(4);
}
