use std::{
    os::unix::{io::AsRawFd, net::UnixStream as OsUnixStream},
    sync::Arc,
};

use bytes::BytesMut;
use log::*;
use parking_lot::RwLock;
use passfd::FdPassingExt;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
    sync::mpsc::{self, Sender},
};
use uuid::Uuid;

use super::hub::ClientRequest;
use super::permissions;
use caro_bus_common::{
    errors::Error as BusError,
    messages::{self, Message, Response, ServiceRequest},
};

type Shared<T> = Arc<RwLock<T>>;

#[derive(Debug)]
enum HubReponse {
    Fd(Message, OsUnixStream),
    Message(Message),
}

#[derive(Clone)]
pub struct Client {
    uuid: Uuid,
    service_name: Shared<String>,
    client_tx: Sender<HubReponse>,
    hub_tx: Sender<ClientRequest>,
}

impl Client {
    #[allow(dead_code)]
    pub fn service_name(&self) -> String {
        self.service_name.read().clone()
    }

    pub fn run(uuid: Uuid, hub_tx: Sender<ClientRequest>, mut socket: UnixStream) -> Self {
        trace!("Starting new client with UUID {:?}", uuid);

        let (client_tx, mut client_rx) = mpsc::channel::<HubReponse>(32);

        let client_handle = Self {
            uuid,
            service_name: Arc::new(RwLock::new(String::from(""))),
            client_tx,
            hub_tx,
        };
        let mut this = client_handle.clone();

        tokio::spawn(async move {
            let mut bytes = BytesMut::with_capacity(64);

            loop {
                tokio::select! {
                    read_result = socket.read_buf(&mut bytes) => {
                        if let Err(err) = read_result {
                            error!("Failed to read from a socket: {}. Client is disconnected. Shutting him down", err.to_string());
                            drop(socket);
                            return
                        }

                        let bytes_read = read_result.unwrap();
                        trace!("Read {} bytes from socket", bytes_read);

                        // Socket closed
                        if bytes_read == 0 {
                            warn!("Client closed socket. Shutting down the connection");

                            drop(socket);
                            return
                        }

                        if let Some(message) = messages::parse_buffer(&mut bytes) {
                            if let Some(response) = this.handle_client_message(message).await {
                                socket.write_all(response.bytes().as_slice()).await.unwrap();
                            }
                        }
                    }
                    Some(outgoing_message) = client_rx.recv() => {
                        // If Err(_) returned, service either closed connection, or we want to close it ourselves
                        if let Err(_) = this.write_response_message(&mut socket, outgoing_message).await {
                            drop(socket);
                            return;
                        }
                    }
                }
            }
        });

        client_handle
    }

    pub async fn send_message(&mut self, service_name: &String, message: Message) {
        debug!(
            "Incoming response message for a service `{}`: {:?}",
            service_name, message
        );

        if let Err(err) = self.client_tx.send(HubReponse::Message(message)).await {
            error!(
                "Failed to send message to the client `{}`: {}",
                service_name,
                err.to_string()
            );
        }
    }

    pub async fn send_connection_fd(
        &mut self,
        counterparty_service_name: &String,
        fd: OsUnixStream,
    ) {
        debug!(
            "Incoming socket descriptor for a service `{}` from `{}`",
            self.service_name(),
            counterparty_service_name
        );

        let message = Response::IncomingClientFd(counterparty_service_name.clone()).into();

        if let Err(err) = self.client_tx.send(HubReponse::Fd(message, fd)).await {
            error!(
                "Failed to send socket descriptor to the client `{}`: {}",
                counterparty_service_name,
                err.to_string()
            );
        }
    }

    async fn write_response_message(
        &mut self,
        socket: &mut UnixStream,
        outgoing_message: HubReponse,
    ) -> std::io::Result<()> {
        match outgoing_message {
            HubReponse::Message(message) => {
                let shutdown = matches!(message, Message::Response(Response::Shutdown));

                if let Err(err) = socket.write_all(message.bytes().as_slice()).await {
                    error!("Failed to write into a socket: {}. Client is disconnected. Shutting him down", err.to_string());
                    return Err(err);
                }

                // Returning error here will drop connection
                if shutdown {
                    return Err(std::io::Error::new(std::io::ErrorKind::ConnectionReset, ""));
                }
            }
            HubReponse::Fd(message, fd) => {
                // With new connections we have two responses:
                // 1. Response::Error, which we receive as a return value from message handle
                // 2. Reponse::Ok, and socket fd right after, which is handled here

                // Send Ok message, so our client starts listening to the incoming fd
                if let Err(err) = socket.write_all(message.bytes().as_slice()).await {
                    error!("Failed to write into a socket: {}. Client is disconnected. Shutting him down", err.to_string());
                    drop(socket);
                    return Err(err);
                }

                // And the descriptor itself. To minimize blocking wait until socket is ready to send
                if let Err(err) = socket.writable().await {
                    error!("Failed to wait for writable socket: {}. Client is disconnected. Shutting him down", err.to_string());
                    drop(socket);
                    return Err(err);
                }

                if let Err(err) = socket.as_raw_fd().send_fd(fd.as_raw_fd()) {
                    error!(
                        "Failed to send fd to the service `{:?}`: {}",
                        self.service_name(),
                        err.to_string()
                    );
                    return Err(err);
                }
            }
        }

        Ok(())
    }

    async fn handle_client_message(&mut self, message: messages::Message) -> Option<Response> {
        trace!(
            "Incoming service `{}` message: {:?}",
            self.service_name.read(),
            message
        );

        if let Message::ServiceRequest(request) = message {
            match request {
                ServiceRequest::Register {
                    protocol_version,
                    service_name,
                } => {
                    self.handle_registration_message(protocol_version, service_name)
                        .await
                }
                ServiceRequest::Connect { service_name } => {
                    self.handle_connect_message(service_name).await
                }
            }
        } else {
            error!("Unexpected data message send to the hub: {:?}", message);
            Some(Response::Error(BusError::InvalidProtocol))
        }
    }

    async fn handle_registration_message(
        &mut self,
        protocol_version: i64,
        service_name: String,
    ) -> Option<Response> {
        if protocol_version != messages::PROTOCOL_VERSION {
            warn!("Client with invalid protocol: {}", self.uuid);
            return Response::Error(BusError::InvalidProtocol).into();
        }

        if !permissions::service_name_allowed(&"socket_addr".into(), &service_name) {
            warn!(
                "Client is not allowed to register with name `{:?}`",
                service_name
            );
            return Response::Error(BusError::InvalidProtocol).into();
        }

        // Service requested new service_name. We update our service name here.
        // In case we've failed to register service, we drop it anyway
        *(self.service_name.write()) = service_name.clone();
        trace!(
            "Assigned service name `{}` to a client with UUID {}",
            self.service_name.read(),
            self.uuid
        );

        self.hub_tx
            .send(ClientRequest {
                uuid: self.uuid.clone(),
                service_name: service_name.clone(),
                message: ServiceRequest::Register {
                    protocol_version,
                    service_name,
                }
                .into(),
            })
            .await
            .unwrap();

        None
    }

    async fn handle_connect_message(&mut self, service_name: String) -> Option<Response> {
        let self_service_name = self.service_name.read().clone();

        if !permissions::connection_allowed(&self_service_name, &service_name) {
            warn!(
                "Client `{:?}` is not allowed to connect with `{:?}`",
                self_service_name, service_name
            );
            return Some(Response::Error(BusError::NotAllowed).into());
        }

        // Notify hub about connection request
        self.hub_tx
            .send(ClientRequest {
                uuid: self.uuid.clone(),
                service_name: self_service_name,
                message: ServiceRequest::Connect { service_name }.into(),
            })
            .await
            .unwrap();

        None
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        debug!(
            "Shutting down service connection for `{:?}`",
            self.service_name.read()
        );

        let _ = self
            .client_tx
            .blocking_send(HubReponse::Message(Response::Shutdown.into()));
    }
}
