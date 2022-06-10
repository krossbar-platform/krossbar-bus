use std::error::Error;
use std::os::unix::net::UnixStream as OsStream;
use std::os::unix::prelude::{AsRawFd, FromRawFd};

use log::*;
use passfd::FdPassingExt;
use tokio::net::UnixStream;

use super::connection::Connection;
use super::utils;
use common::errors::Error as BusError;
use common::messages::{self, Message, Response};
use common::HUB_SOCKET_PATH;

pub struct BusClient {
    service_name: String,
    socket: UnixStream,
}

impl BusClient {
    pub async fn new(service_name: String) -> std::io::Result<Self> {
        debug!("Performing hub connection at: {}", HUB_SOCKET_PATH);

        let socket = UnixStream::connect(HUB_SOCKET_PATH).await?;

        Ok(Self {
            service_name,
            socket,
        })
    }

    pub async fn register(&mut self) -> Result<(), Box<dyn Error>> {
        debug!("Registering service `{}`", self.service_name);
        let request = messages::make_register_message(self.service_name.clone());

        match utils::send_receive(&mut self.socket, request).await {
            Ok(Message::Response(Response::Ok)) => {
                info!("Succesfully registered service as `{}`", self.service_name);
                Ok(())
            }
            Ok(Message::Response(Response::Error(err))) => {
                warn!(
                    "Failed to register service as `{}`: {}",
                    self.service_name, err
                );
                Err(Box::new(err))
            }
            Ok(m) => {
                error!("Invalid response from the hub: {:?}", m);
                Err(Box::new(BusError::InvalidProtocol))
            }
            Err(err) => Err(err),
        }
    }

    pub async fn connect(&mut self, service_name: String) -> Result<Connection, Box<dyn Error>> {
        debug!("Connection to a service `{}`", service_name);
        let request = messages::make_connection_message(service_name.clone());

        match utils::send_receive(&mut self.socket, request).await {
            Ok(Message::Response(Response::Ok)) => {
                info!(
                    "Succesfully connected service `{}` to `{}`",
                    self.service_name, service_name
                );
                // Client can receive two types of responses for a connection request:
                // 1. Response::Error if servic eis not allowed to connect
                // 2. Response::Ok and fd right after it if the hub allows the connection
                // We handle second option here

                let fd = self.socket.as_raw_fd().recv_fd()?;
                let os_stream = unsafe { OsStream::from_raw_fd(fd) };
                let socket = UnixStream::from_std(os_stream).unwrap();
                Ok(Connection::new(service_name, socket))
            }
            Ok(Message::Response(Response::Error(err))) => {
                warn!(
                    "Failed to register service as `{}`: {}",
                    self.service_name, err
                );
                Err(Box::new(err))
            }
            Ok(m) => {
                error!("Invalid response from the hub: {:?}", m);
                Err(Box::new(BusError::InvalidProtocol))
            }
            Err(err) => Err(err),
        }
    }
}
