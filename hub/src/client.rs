use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream as OsUnixStream;
use std::sync::Arc;

use bson;
use bytes::BytesMut;
use log::*;
use parking_lot::RwLock;
use sendfd::SendWithFd;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::select;
use tokio::sync::mpsc::{self, Sender};
use uuid::Uuid;

use super::hub::ClientRequest;
use super::permissions;
use messages::{self, Message, Response, ServiceRequest};

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
                select! {
                    _ = socket.read_buf(&mut bytes) => {
                        if let Some(message) = messages::parse_buffer(&mut bytes) {
                            if let Some(response) = this.handle_client_message(message).await {
                                socket.write_all(response.bytes().as_slice()).await.unwrap();
                            }
                        }
                    }
                    Some(outgoing_message) = client_rx.recv() => {
                        match outgoing_message {
                            HubReponse::Message(message) => {
                                let shutdown = matches!(message, Message::Response(Response::Shutdown));

                                socket.write_all(message.bytes().as_slice()).await.unwrap();

                                if shutdown {
                                    drop(socket);
                                    return
                                }
                            },
                            HubReponse::Fd(message, fd) => {
                                let bson = bson::to_raw_document_buf(&message).unwrap().into_bytes();
                                let fds = vec![fd.as_raw_fd()];

                                if let Err(err) = socket.send_with_fd(bson.as_slice(), fds.as_slice()) {
                                    error!("Failed to send fd to the service `{:?}`: {}", this.service_name(), err.to_string());
                                }
                            }
                        }

                    }
                }
            }
        });

        client_handle
    }

    pub async fn send_message(&mut self, service_name: &String, message: Message) {
        debug!(
            "Incoming response message for a service `{:?}`: {:?}",
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

    pub async fn send_new_connection_descriptor(
        &mut self,
        counterparty_service_name: &String,
        fd: OsUnixStream,
    ) {
        debug!(
            "Incoming socket descriptor for a service `{:?}` from `{:?}`",
            self.service_name(),
            counterparty_service_name
        );

        let message = Response::NewConnection(counterparty_service_name.clone()).into();

        if let Err(err) = self.client_tx.send(HubReponse::Fd(message, fd)).await {
            error!(
                "Failed to send socket descriptor to the client `{}`: {}",
                counterparty_service_name,
                err.to_string()
            );
        }
    }

    async fn handle_client_message(&mut self, message: messages::Message) -> Option<Response> {
        trace!(
            "Incoming service `{:?}` message: {:?}",
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
            Some(Response::InvalidProtocol)
        }
    }

    async fn handle_registration_message(
        &mut self,
        protocol_version: i64,
        service_name: String,
    ) -> Option<Response> {
        if protocol_version != messages::PROTOCOL_VERSION {
            warn!("Client with invalid protocol: {}", self.uuid);
            return Response::InvalidProtocol.into();
        }

        if !permissions::service_name_allowed(&"socket_addr".into(), &service_name) {
            warn!(
                "Client is not allowed to register with name `{:?}`",
                service_name
            );
            return Response::NotAllowed.into();
        }

        // Service requested new service_name. We update our service name here.
        // In case we've failed to register service, we drop it anyway
        *(self.service_name.write()) = service_name.clone();
        trace!(
            "Assigned service name `{:?}` to a client with UUID {:?}",
            self.service_name,
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
            return Some(Response::NotAllowed.into());
        }

        self.hub_tx
            .send(ClientRequest {
                uuid: self.uuid.clone(),
                service_name: service_name.clone(),
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
