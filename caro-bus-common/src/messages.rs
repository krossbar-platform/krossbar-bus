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

#[derive(Serialize, Deserialize, Debug)]
pub struct Message {
    pub(crate) seq: u64,
    pub(crate) body: MessageBody,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum MessageBody {
    ServiceRequest(ServiceRequest),
    Response(Response),
    MethodCall { method_name: String, params: Bson },
    MethodSubscription { method_name: String },
}

impl IntoMessage for MessageBody {
    fn into_message(self, seq: u64) -> Message {
        Message { seq, body: self }
    }
}

impl Message {
    pub fn body(&self) -> &MessageBody {
        &self.body
    }

    pub fn seq(&self) -> u64 {
        self.seq
    }

    pub fn bytes(self) -> Vec<u8> {
        bson::to_raw_document_buf(&self).unwrap().into_bytes()
    }

    pub fn new_registration(service_name: String) -> Self {
        Self {
            seq: INVALID_SEQ,
            body: MessageBody::ServiceRequest(ServiceRequest::Register {
                protocol_version: PROTOCOL_VERSION,
                service_name,
            }),
        }
    }

    pub fn new_connection(peer_service_name: String) -> Self {
        Self {
            seq: INVALID_SEQ,
            body: MessageBody::ServiceRequest(ServiceRequest::Connect { peer_service_name }),
        }
    }

    pub fn new_call<T: Serialize>(method_name: String, data: &T) -> Self {
        Self {
            seq: INVALID_SEQ,
            body: MessageBody::MethodCall {
                method_name,
                params: bson::to_bson(&data).unwrap(),
            },
        }
    }

    pub fn new_subscription(method_name: String) -> Self {
        Self {
            seq: INVALID_SEQ,
            body: MessageBody::MethodSubscription { method_name },
        }
    }
}

pub enum EitherMessage {
    FullMessage(Message),
    NeedMoreData(usize),
}

#[derive(Serialize, Deserialize, Debug)]
pub enum ServiceRequest {
    Register {
        protocol_version: i64,
        service_name: String,
    },
    Connect {
        peer_service_name: String,
    },
}

impl IntoMessage for ServiceRequest {
    fn into_message(self, seq: u64) -> Message {
        Message {
            seq,
            body: MessageBody::ServiceRequest(self),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Response {
    Ok,
    Shutdown,
    IncomingClientFd(String),
    Return(Bson),
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

/// Try to parse a Message from the buffer
/// *Returns*:
/// 1. EitherMessage::FullMessage(message) if completely read the message
/// 2. EitherMessage::NeedMoreData(len) if still need to read n bytes of data to get a message
pub(crate) fn parse_buffer(buffer: &mut BytesMut) -> EitherMessage {
    // Not enough data
    if buffer.len() < 4 {
        return EitherMessage::NeedMoreData(4);
    }

    match buffer.as_ref()[0..4].try_into() {
        Ok(frame_len_bytes) => {
            // First lets find BSON document len
            let frame_len = i32::from_le_bytes(frame_len_bytes) as usize;

            trace!("Next frame len: {}. Buffer len {}", frame_len, buffer.len());

            // If buffer contains complete document, try to parse
            if buffer.len() >= frame_len {
                if let Ok(frame) = bson::from_slice(buffer.as_ref()) {
                    buffer.advance(frame_len);

                    trace!("Incoming BSON message of len {}: {:?}", frame_len, frame);

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
            error!("Failed to convert buffer in a slice: {}", err);
        }
    }

    return EitherMessage::NeedMoreData(4);
}
