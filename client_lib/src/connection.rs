use std::error::Error;

use log::*;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::net::UnixStream;

use super::utils;
use common::errors::Error as BusError;
use common::messages::{self, Message, Response};

pub struct Connection {
    connection_service_name: String,
    socket: UnixStream,
}

impl Connection {
    pub fn new(connection_service_name: String, socket: UnixStream) -> Self {
        Self {
            connection_service_name,
            socket,
        }
    }

    pub fn service_name(&self) -> &String {
        &self.connection_service_name
    }

    pub async fn call<'de, ParamsType: Serialize, ResponseType: DeserializeOwned>(
        &mut self,
        method_name: &str,
        params: &ParamsType,
    ) -> Result<ResponseType, Box<dyn Error>> {
        let message = messages::make_call_message(method_name, params);

        let response = utils::send_receive(&mut self.socket, message).await?;

        match response {
            Message::Response(Response::Return(data)) => {
                match bson::from_bson::<ResponseType>(data) {
                    Ok(data) => Ok(data),
                    Err(err) => {
                        error!("Can't deserialize method response: {}", err.to_string());
                        Err(Box::new(BusError::InvalidResponse))
                    }
                }
            }
            Message::Response(Response::Error(err)) => {
                warn!(
                    "Failed to perform a call to `{}::{}`: {}",
                    self.connection_service_name,
                    method_name,
                    err.to_string()
                );
                Err(Box::new(err))
            }
            r => {
                error!("Ivalid response from a method call: {:?}", r);
                Err(Box::new(BusError::InvalidProtocol))
            }
        }
    }
}
