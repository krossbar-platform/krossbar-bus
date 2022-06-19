use std::{collections::HashMap, fs, os::unix::net::UnixStream as OsUnixStream};

use caro_bus_common::{
    errors::Error as BusError,
    messages::{IntoMessage, Message, MessageBody, Response, ServiceRequest},
    HUB_SOCKET_PATH,
};
use log::*;
use tokio::{
    net::{UnixListener, UnixStream},
    sync::mpsc::{self, Receiver, Sender},
};
use uuid::Uuid;

use super::client::Client;

#[derive(Debug)]
pub struct ClientRequest {
    pub uuid: Uuid,
    pub service_name: String,
    pub message: Message,
}

pub struct Hub {
    client_tx: Sender<ClientRequest>,
    hub_rx: Receiver<ClientRequest>,
    shutdown_rx: Receiver<()>,
    anonymous_clients: HashMap<Uuid, Client>,
    clients: HashMap<String, Client>,
}

impl Hub {
    pub fn new(shutdown_rx: Receiver<()>) -> Self {
        let (client_tx, hub_rx) = mpsc::channel::<ClientRequest>(32);

        Self {
            client_tx,
            hub_rx,
            shutdown_rx,
            anonymous_clients: HashMap::new(),
            clients: HashMap::new(),
        }
    }

    pub async fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        match UnixListener::bind(HUB_SOCKET_PATH) {
            Ok(listener) => {
                info!(
                    "Succesfully started listening for incoming connections at: {}",
                    HUB_SOCKET_PATH
                );

                loop {
                    tokio::select! {
                        Ok((socket, address)) = listener.accept() => {
                            info!("New connection from a binary {:?}", address.as_pathname());

                            self.handle_new_client(socket).await
                        },
                        Some(client_message) = self.hub_rx.recv() => {
                            trace!("Incoming client call: {:?}", client_message);

                            self.handle_client_call(client_message).await
                        }
                        _ = self.shutdown_rx.recv() => {
                            drop(listener);
                            return Ok(());
                        }
                    }
                }
            }
            Err(err) => {
                error!(
                    "Failed to start listening at: {}. Another hub instance is running?",
                    HUB_SOCKET_PATH
                );
                return Err(Box::new(err));
            }
        }
    }

    fn client(&mut self, service_name: &String) -> &mut Client {
        self.clients.get_mut(service_name).unwrap()
    }

    async fn handle_new_client(&mut self, socket: UnixStream) {
        // Temporal name until client sends registration message
        let uuid = Uuid::new_v4();

        let client = Client::run(uuid.clone(), self.client_tx.clone(), socket);

        self.anonymous_clients.insert(uuid.clone(), client);
    }

    async fn handle_client_call(&mut self, request: ClientRequest) {
        match request.message.body() {
            MessageBody::ServiceRequest(ServiceRequest::Register {
                protocol_version: _,
                service_name,
            }) => {
                self.handle_client_registration(
                    request.uuid,
                    service_name.clone(),
                    request.message.seq(),
                )
                .await
            }
            MessageBody::ServiceRequest(ServiceRequest::Connect { peer_service_name }) => {
                self.handle_new_connection_request(
                    request.service_name,
                    peer_service_name.clone(),
                    request.message.seq(),
                )
                .await
            }
            MessageBody::Response(Response::Shutdown) => {
                self.handle_disconnection(&request.uuid, &request.service_name)
                    .await;
            }
            message => {
                error!(
                    "Ivalid message from a client `{:?}`: {:?}",
                    request.service_name, message
                );
            }
        }
    }

    async fn handle_client_registration(&mut self, uuid: Uuid, service_name: String, seq: u64) {
        trace!(
            "Trying to assign service name `{}` to a client with uuid {}",
            service_name,
            uuid
        );

        match self.anonymous_clients.remove(&uuid) {
            Some(mut client) => {
                if self.clients.contains_key(&service_name) {
                    error!(
                        "Failed to register a client with name `{}`. Already exists",
                        service_name
                    );

                    client
                        .send_message(&service_name, BusError::NameRegistered.into_message(seq))
                        .await;
                } else {
                    info!("Succesfully registered new client: `{}`", service_name);

                    client
                        .send_message(&service_name, Response::Ok.into_message(seq))
                        .await;
                    self.clients.insert(service_name, client);

                    trace!("New named clients count: {}", self.clients.len());
                }
            }
            e => {
                error!(
                    "Failed to find a client `{}`, which tries to register. This should never happen",
                    uuid
                );
                e.unwrap();
            }
        }
    }

    async fn handle_new_connection_request(
        &mut self,
        client_service_name: String,
        target_service_name: String,
        seq: u64,
    ) {
        trace!(
            "Trying to connect `{}` to the {}",
            client_service_name,
            target_service_name
        );

        let (left, right) = OsUnixStream::pair().unwrap();

        {
            // Service to which our client wants to connect is not registered
            if !self.clients.contains_key(&target_service_name) {
                warn!(
                    "Failed to find a service `{:?}` to connect with `{:?}`",
                    target_service_name, client_service_name
                );

                self.client(&client_service_name)
                    .send_message(&target_service_name, BusError::NotFound.into_message(seq))
                    .await;
                return;
            }

            if client_service_name == target_service_name {
                warn!(
                    "Service `{:?}` tries to connect to himself",
                    target_service_name,
                );

                self.client(&client_service_name)
                    .send_message(&target_service_name, BusError::NotAllowed.into_message(seq))
                    .await;
                return;
            }

            // Send descriptor to the requester
            self.client(&client_service_name)
                .send_connection_fd(&target_service_name, left)
                .await;
        }

        // Send descriptor to the target service
        self.client(&target_service_name)
            .send_connection_fd(&client_service_name, right)
            .await;

        info!(
            "Succesfully connected `{}` to `{}`",
            client_service_name, target_service_name
        )
    }

    async fn handle_disconnection(&mut self, uuid: &Uuid, service_name: &String) {
        self.anonymous_clients.remove(uuid);
        self.clients.remove(service_name);

        trace!("New named clients count: {}", self.clients.len());
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        if let Err(err) = fs::remove_file(HUB_SOCKET_PATH) {
            error!("Failed to remove hub socket file: {}", err);
        }
    }
}
