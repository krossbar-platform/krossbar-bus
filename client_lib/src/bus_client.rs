use std::error::Error;
use std::os::unix::io::RawFd;
use std::os::unix::net::UnixStream as OsStream;
use std::os::unix::prelude::FromRawFd;

use bytes::BytesMut;
use log::*;
use sendfd::RecvWithFd;
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

        utils::send(&mut self.socket, request).await?;

        //let mut buffer = [0u8; 1024];
        let mut bytes = BytesMut::with_capacity(512);
        let mut fd_buffer: [RawFd; 1] = [0];

        match self.socket.recv_with_fd(&mut bytes, &mut fd_buffer) {
            Ok((n, fd_size)) => {
                /*match common::messages::parse_buffer(bytes) {
                    Some(message) => {

                    }
                }*/

                info!(
                    "Succesfully connected service `{}` to `{}`",
                    self.service_name, service_name
                );

                let os_stream = unsafe { OsStream::from_raw_fd(fd_buffer[0]) };
                let socket = UnixStream::from_std(os_stream).unwrap();
                Ok(Connection::new(service_name, socket))
            }
            Err(err) => Err(Box::new(err)),
        }
    }
}
