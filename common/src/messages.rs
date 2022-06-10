use bson::{self, Bson};
use bytes::{Buf, BytesMut};
use log::*;
use serde::{Deserialize, Serialize};

use super::errors;

pub const PROTOCOL_VERSION: i64 = 1;

#[derive(Serialize, Deserialize, Debug)]
pub enum ServiceRequest {
    Register {
        protocol_version: i64,
        service_name: String,
    },
    Connect {
        service_name: String,
    },
}

impl Into<Message> for ServiceRequest {
    fn into(self) -> Message {
        Message::ServiceRequest(self)
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Response {
    Ok,
    Shutdown,
    Return(Bson),
    Error(errors::Error),
}

impl Into<Message> for Response {
    fn into(self) -> Message {
        Message::Response(self)
    }
}

impl Response {
    pub fn bytes(self) -> Vec<u8> {
        let message: Message = self.into();
        message.bytes()
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Message {
    ServiceRequest(ServiceRequest),
    Response(Response),
    MethodCall { method_name: String, params: Bson },
    MethodSubscription { method_name: String },
}

impl Message {
    pub fn bytes(self) -> Vec<u8> {
        bson::to_raw_document_buf(&self).unwrap().into_bytes()
    }
}

pub fn make_register_message(service_name: String) -> Message {
    Message::ServiceRequest(ServiceRequest::Register {
        protocol_version: PROTOCOL_VERSION,
        service_name,
    })
}

pub fn make_connection_message(service_name: String) -> Message {
    Message::ServiceRequest(ServiceRequest::Connect { service_name })
}

pub fn make_call_message<T: Serialize>(method_name: &str, data: &T) -> Message {
    Message::MethodCall {
        method_name: String::from(method_name),
        params: bson::to_bson(&data).unwrap(),
    }
}

pub fn make_subscription_message(method_name: &str) -> Message {
    Message::MethodSubscription {
        method_name: String::from(method_name),
    }
}

pub fn parse_buffer(buffer: &mut BytesMut) -> Option<Message> {
    if let Ok(frame_len_bytes) = buffer.as_ref().try_into() {
        // First lets find BSON document len
        let frame_len = i32::from_le_bytes(frame_len_bytes) as usize;

        trace!("Trying to parse frame with len: {}", frame_len);

        // If buffer contains complete document, try to parse
        if buffer.len() >= frame_len {
            if let Ok(frame) = bson::from_slice(buffer.as_ref()) {
                buffer.advance(frame_len);

                trace!("Incoming BSON message of len: {}", frame_len);

                return Some(frame);
            } else {
                // TODO: Recover
            }
        }
    }

    None
}
