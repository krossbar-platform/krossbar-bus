use std::{
    io::ErrorKind,
    os::unix::{io::AsRawFd, net::UnixStream as OsUnixStream},
    sync::Arc,
};

use bytes::BytesMut;
use log::*;
use parking_lot::RwLock;
use passfd::tokio::FdPassingExt;
use tokio::{
    io::AsyncWriteExt,
    net::UnixStream,
    sync::mpsc::{self, Receiver, Sender},
};
use uuid::Uuid;

use super::hub::ClientRequest;
use super::permissions;
use caro_bus_common::{
    errors::Error as BusError,
    messages::{self, IntoMessage, Message, MessageBody, Response, ServiceRequest},
    net,
};

type Shared<T> = Arc<RwLock<T>>;

#[derive(Debug)]
enum HubReponse {
    Fd(Message, OsUnixStream),
    Message(Message),
    Shutdown(Message),
}

#[derive(Clone)]
pub struct Client {
    uuid: Uuid,
    service_name: Shared<String>,
    task_tx: Sender<HubReponse>,
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
            task_tx: client_tx,
            hub_tx,
        };
        let mut this = client_handle.clone();

        tokio::spawn(async move {
            let mut bytes = BytesMut::with_capacity(64);

            loop {
                tokio::select! {
                    read_result = net::read_message(&mut socket, &mut bytes) => {
                        match read_result {
                            Ok(message) => {
                                if let Some(response) = this.handle_client_request(message).await {
                                    // Failed to write into socket. Client shutwodn
                                    if let Err(err) = socket.write_all(response.bytes().as_slice()).await {
                                        error!("Failed to write into a client socket: {}. Shutting him down", err.to_string());
                                        // NOTE: We do not drop socket here. First we ask hub to delete connection handler
                                        // And later drop routing will send us a message to close connection and return
                                        // See client_tx handling
                                        this.perform_shutdown(&mut socket, &mut client_rx).await;
                                        drop(socket);
                                        return
                                    }
                                }},
                            Err(err) => {
                                warn!("Client closed socket. Asking hub to delete the connection becasue of: {}", err.to_string());

                                // NOTE: We do not drop socket here. First we ask hub to delete connection handler
                                // And later drop routing will send us a message to close connection and return
                                // See client_tx handling
                                this.perform_shutdown(&mut socket, &mut client_rx).await;
                                drop(socket);
                                return
                            }
                        }
                    },
                    Some(outgoing_message) = client_rx.recv() => {
                        trace!("Outgoing client message: {:?}", outgoing_message);

                        // If Err(_) returned, hub wants to close our connection
                        if let Err(_) = this.write_response_message(&mut socket, outgoing_message).await {
                            drop(socket);
                            return
                        }
                    }
                }
            }
        });

        client_handle
    }

    // Request to send message to a client
    pub async fn send_message(&mut self, service_name: &String, message: Message) {
        debug!(
            "Incoming response message for a service `{}`: {:?}",
            service_name, message
        );

        if let Err(err) = self.task_tx.send(HubReponse::Message(message)).await {
            error!(
                "Failed to send message to the client `{}`: {}",
                service_name,
                err.to_string()
            );
        }
    }

    // Request to send connection fd to a client
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

        let message =
            Response::IncomingClientFd(counterparty_service_name.clone()).into_message(999);

        if let Err(err) = self.task_tx.send(HubReponse::Fd(message, fd)).await {
            error!(
                "Failed to send socket descriptor to the client `{}`: {}",
                counterparty_service_name,
                err.to_string()
            );
        }
    }

    // Write response message into a client socket
    async fn write_response_message(
        &mut self,
        socket: &mut UnixStream,
        outgoing_message: HubReponse,
    ) -> std::io::Result<()> {
        match outgoing_message {
            HubReponse::Message(message) => {
                let shutdown = matches!(message.body(), MessageBody::Response(Response::Shutdown));

                if let Err(err) = socket.write_all(message.bytes().as_slice()).await {
                    error!("Failed to write into a socket: {}. Client is disconnected. Shutting him down", err.to_string());
                    return Err(err);
                }

                trace!("Successfully sent message to `{}`", self.service_name());

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
                    return Err(err);
                }

                trace!(
                    "Got client socket to send to a service `{}`. Trying to send",
                    self.service_name()
                );

                if let Err(err) = socket.send_fd(fd.as_raw_fd()).await {
                    error!(
                        "Failed to send fd to the service `{:?}`: {}",
                        self.service_name(),
                        err.to_string()
                    );
                    return Err(err);
                }

                debug!("Successfully sent peer socket to `{}`", self.service_name());
            }
            HubReponse::Shutdown(message) => {
                info!(
                    "Hub wants to close connection. Shutting down `{}`",
                    self.service_name()
                );
                let _ = socket.write_all(message.bytes().as_slice()).await;

                return Err(ErrorKind::ConnectionReset.into());
            }
        }

        Ok(())
    }

    // Handle incoming client message
    async fn handle_client_request(&mut self, message: messages::Message) -> Option<Message> {
        trace!(
            "Incoming service `{}` message: {:?}",
            self.service_name.read(),
            message
        );

        if let MessageBody::ServiceRequest(request) = message.body() {
            match request {
                ServiceRequest::Register {
                    protocol_version,
                    service_name,
                } => {
                    self.handle_registration_message(
                        *protocol_version,
                        service_name.clone(),
                        message.seq(),
                    )
                    .await
                }
                ServiceRequest::Connect { peer_service_name } => {
                    self.handle_connect_message(peer_service_name.clone(), message.seq())
                        .await
                }
            }
        } else {
            error!("Unexpected data message send to the hub: {:?}", message);
            Some(BusError::InvalidProtocol.into_message(message.seq()))
        }
    }

    // Handle incoming client registration request
    async fn handle_registration_message(
        &mut self,
        protocol_version: i64,
        service_name: String,
        seq: u64,
    ) -> Option<Message> {
        if protocol_version != messages::PROTOCOL_VERSION {
            warn!("Client with invalid protocol: {}", self.uuid);
            return Some(BusError::InvalidProtocol.into_message(seq));
        }

        if !permissions::service_name_allowed(&"socket_addr".into(), &service_name) {
            warn!(
                "Client is not allowed to register with name `{:?}`",
                service_name
            );
            return Some(BusError::InvalidProtocol.into_message(seq));
        }

        // Service requested new service_name. We update our service name here.
        // In case we've failed to register service, we drop it anyway
        *(self.service_name.write()) = service_name.clone();
        trace!(
            "Assigned service name `{}` to a client with UUID {}",
            self.service_name.read(),
            self.uuid
        );

        self.send_message_to_hub(
            ServiceRequest::Register {
                protocol_version,
                service_name,
            }
            .into_message(seq),
        )
        .await;

        None
    }

    // handle incoming client connection request
    async fn handle_connect_message(
        &mut self,
        peer_service_name: String,
        seq: u64,
    ) -> Option<Message> {
        let self_service_name = self.service_name.read().clone();

        if !permissions::connection_allowed(&self_service_name, &peer_service_name) {
            warn!(
                "Client `{:?}` is not allowed to connect with `{:?}`",
                self_service_name, peer_service_name
            );
            return Some(BusError::NotAllowed.into_message(seq));
        }

        // Notify hub about connection request
        self.send_message_to_hub(ServiceRequest::Connect { peer_service_name }.into_message(seq))
            .await;

        None
    }

    // Sends a message to the hub through a channel
    async fn send_message_to_hub(&self, message: Message) {
        let service_name = self.service_name.read().clone();

        self.hub_tx
            .send(ClientRequest {
                uuid: self.uuid.clone(),
                service_name,
                message: message,
            })
            .await
            .unwrap();
    }

    async fn perform_shutdown(&mut self, socket: &mut UnixStream, rx: &mut Receiver<HubReponse>) {
        trace!("Starting shutdown sequence for a client");

        // Send request to a hub, asking to delete client handle
        self.send_message_to_hub(Response::Shutdown.into_message(999))
            .await;
        // Do not perform any IO, but wait for a response from the hub
        let message = rx.recv().await.unwrap();
        // Try to send response to a client, if he's still alive
        let _ = self.write_response_message(socket, message).await;

        trace!("Finished shutdown sequence for a client");
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if self.task_tx.is_closed() {
            return;
        }

        debug!(
            "Shutting down service connection for `{:?}`",
            self.service_name.read()
        );

        let tx = self.task_tx.clone();
        tokio::spawn(async move {
            tx.send(HubReponse::Shutdown(Response::Shutdown.into_message(999)))
                .await
                .unwrap();
        });
    }
}
