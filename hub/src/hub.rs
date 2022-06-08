use std::collections::HashMap;
use std::os::unix::net::UnixStream as OsUnixStream;

use log::*;
use messages::{Message, ServiceRequest, Response};
use tokio::net::{UnixListener, UnixStream};
use tokio::select;
use tokio::sync::mpsc::{self, Receiver, Sender};
use uuid::Uuid;

use super::client::Client;

const SOCKET_PATH: &str = "/var/run/caro/bus.socket";

#[derive(Debug)]
pub struct ClientRequest {
    pub uuid: Uuid,
    pub service_name: String,
    pub message: Message,
}

pub struct Hub {
    client_tx: Sender<ClientRequest>,
    hub_rx: Receiver<ClientRequest>,
    anonymous_clients: HashMap<Uuid, Client>,
    clients: HashMap<String, Client>,
}

impl Hub {
    pub fn new() -> Self {
        let (client_tx, hub_rx) = mpsc::channel::<ClientRequest>(32);

        Self {
            client_tx,
            hub_rx,
            anonymous_clients: HashMap::new(),
            clients: HashMap::new(),
        }
    }

    pub async fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        match UnixListener::bind(SOCKET_PATH) {
            Ok(listener) => {
                info!(
                    "Succesfully started listening for incoming connections at: {}",
                    SOCKET_PATH
                );

                loop {
                    select! {
                        Ok((socket, address)) = listener.accept() => {
                            info!("New connection from a binary {:?}", address.as_pathname());

                            self.handle_new_client(socket).await
                        },
                        Some(client_message) = self.hub_rx.recv() => {
                            trace!("Incoming client message: {:?}", client_message);

                            self.handle_client_message(client_message).await
                        }
                    }
                }
            }
            Err(err) => {
                error!(
                    "Failed to start listening at: {}. Another hub instance is running?",
                    SOCKET_PATH
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

    async fn handle_client_message(&mut self, request: ClientRequest) {
        if let Message::ServiceRequest(service_message) = request.message {
            match service_message {
                ServiceRequest::Register {
                    protocol_version: _,
                    service_name,
                } => {
                    self.handle_client_registration(request.uuid, service_name).await
                },
                ServiceRequest::Connect { service_name } => {
                    self.handle_new_connection_request(request.service_name,
                         service_name).await
                }
            }
        } else {
            error!(
                "Ivalid message from a client `{:?}`: {:?}",
                request.service_name, request.message
            );
        }
    }

    async fn handle_client_registration(&mut self, uuid: Uuid, service_name: String) {
        match self.anonymous_clients.remove(&uuid) {
            Some(mut client) => {
                if self.clients.contains_key(&service_name) {
                    error!(
                        "Failed to register a client with name `{}`. Already exists",
                        service_name
                    );

                    client
                        .drop(&service_name, Response::NameRegistered)
                        .await;
                } else {
                    self.clients.insert(service_name, client);
                }
            }
            _ => error!(
                "Failed to find a client `{}`, which tries to register",
                uuid
            ),
        }
    }

    async fn handle_new_connection_request(&mut self, client_service_name: String, target_service_name: String) {
        let (left, right) = OsUnixStream::pair().unwrap();

        {
            // Service to which our client wants to connect is not registered
            if !self.clients.contains_key(&target_service_name) {
                warn!(
                    "Failed to find a service `{:?}` to connect with `{:?}`",
                    target_service_name, client_service_name
                );

                self.client(&client_service_name)
                    .send_message(&target_service_name, Response::NotFound.into())
                    .await;
                return;
            }

            // Send descriptor to the requester
            self.client(&client_service_name)
                .send_new_connection_descriptor(&target_service_name, left)
                .await;
        }

        // Send descriptor to the target service
        self.client(&target_service_name)
            .send_new_connection_descriptor(&client_service_name, right)
            .await;

        info!(
            "Connected a service `{:?}` to `{:?}`", client_service_name, target_service_name)
    }
}
