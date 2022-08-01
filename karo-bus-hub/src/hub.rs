use std::{collections::HashMap, fs, os::unix::prelude::PermissionsExt, sync::Arc};

use karo_bus_common::{
    self as common,
    errors::Error as BusError,
    messages::{IntoMessage, Message, MessageBody, Response, ServiceMessage},
};
use log::*;
use tokio::{
    net::{UnixListener, UnixStream},
    sync::mpsc::{self, Receiver, Sender},
};
use uuid::Uuid;

use crate::{args::Args, client::Client, permissions::Permissions};

struct PendingConnectionRequest {
    requester_service_name: String,
    request: Message,
}

/// Incoming client request
#[derive(Debug)]
pub struct ClientRequest {
    /// Client unique id
    pub uuid: Uuid,
    /// Client service name. Can be empty if not registered yet
    pub service_name: String,
    /// Request message
    pub message: Message,
}

pub struct Hub {
    /// Sender to recieve requests from th eclients
    client_tx: Sender<ClientRequest>,
    /// Receiver to recieve requests from th eclients
    hub_rx: Receiver<ClientRequest>,
    /// Receiver to listen for shutdown requests
    shutdown_rx: Receiver<()>,
    /// A map of anonymous clients. Once a client is registered, it's moved into [Hub::clients]
    anonymous_clients: HashMap<Uuid, Client>,
    /// A map of laready registered clients
    clients: HashMap<String, Client>,
    /// Permissions handle
    permissions: Arc<Permissions>,
    /// If a client uses 'Bus::connect_await' from Karo lib, it's waiting for a peer connection in this map
    pending_connections: HashMap<String, Vec<PendingConnectionRequest>>,
}

impl Hub {
    pub fn new(args: Args, shutdown_rx: Receiver<()>) -> Self {
        let (client_tx, hub_rx) = mpsc::channel::<ClientRequest>(32);

        Self {
            client_tx,
            hub_rx,
            shutdown_rx,
            anonymous_clients: HashMap::new(),
            clients: HashMap::new(),
            permissions: Arc::new(Permissions::new(&args.service_files_dir)),
            pending_connections: HashMap::new(),
        }
    }

    /// Start listening for incoming connections
    pub async fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let socket_path = common::get_hub_socket_path();

        match UnixListener::bind(socket_path.clone()) {
            Ok(listener) => {
                info!(
                    "Succesfully started listening for incoming connections at: {}",
                    socket_path
                );

                // Update permissions to be accessible for th eclient
                let socket_permissions = fs::Permissions::from_mode(0o666);
                fs::set_permissions(socket_path.clone(), socket_permissions)?;

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
                    socket_path
                );
                return Err(Box::new(err));
            }
        }
    }

    fn client(&mut self, service_name: &String) -> &mut Client {
        self.clients.get_mut(service_name).unwrap()
    }

    /// Handle new connection
    async fn handle_new_client(&mut self, socket: UnixStream) {
        // Temporal ID until client sends registration message
        let uuid = Uuid::new_v4();

        let client = Client::run(
            uuid.clone(),
            self.client_tx.clone(),
            socket,
            self.permissions.clone(),
        );

        self.anonymous_clients.insert(uuid.clone(), client);
    }

    /// Handle a message from a client
    async fn handle_client_call(&mut self, request: ClientRequest) {
        match request.message.body() {
            MessageBody::ServiceMessage(ServiceMessage::Register { .. }) => {
                self.handle_client_registration(request.uuid, request.message)
                    .await
            }
            MessageBody::ServiceMessage(ServiceMessage::Connect { .. }) => {
                self.handle_new_connection_request(request.service_name, request.message)
                    .await
            }
            MessageBody::Response(Response::Shutdown(_)) => {
                self.handle_client_disconnection(&request.uuid, &request.service_name)
                    .await;
            }
            message => {
                error!(
                    "Ivalid message from a client `{}`: {:?}",
                    request.service_name, message
                );
            }
        }
    }

    /// Hadnle registration message from a client
    async fn handle_client_registration(&mut self, uuid: Uuid, request: Message) {
        let (_, service_name) = match request.body() {
            MessageBody::ServiceMessage(ServiceMessage::Register {
                protocol_version,
                service_name,
            }) => (protocol_version, service_name),
            _ => panic!("Should never happen"),
        };

        trace!(
            "Trying to assign service name `{}` to a client with uuid {}",
            service_name,
            uuid
        );

        match self.anonymous_clients.remove(&uuid) {
            Some(mut client) => {
                if self.clients.contains_key(service_name) {
                    error!(
                        "Failed to register a client with name `{}`. Already exists",
                        service_name
                    );

                    client
                        .send_message(
                            &service_name,
                            BusError::NameRegistered.into_message(request.seq()),
                        )
                        .await;
                } else {
                    info!("Succesfully registered new client: `{}`", service_name);

                    client
                        .send_message(&service_name, Response::Ok.into_message(request.seq()))
                        .await;
                    self.clients.insert(service_name.clone(), client);

                    // Check if we have pending connections to the client.
                    // If we do, we resolve all connection request by sending response
                    if let Some(pending_connection_requests) =
                        self.pending_connections.remove(service_name)
                    {
                        for request in pending_connection_requests {
                            trace!(
                                "Resolving connection request to {} from {}",
                                service_name,
                                request.requester_service_name
                            );

                            self.handle_new_connection_request(
                                request.requester_service_name,
                                request.request,
                            )
                            .await;
                        }
                    }

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

    /// Handle peer connection message from a client
    async fn handle_new_connection_request(
        &mut self,
        requester_service_name: String,
        request: Message,
    ) {
        let (target_service_name, await_connection) = match request.body() {
            MessageBody::ServiceMessage(ServiceMessage::Connect {
                peer_service_name,
                await_connection,
            }) => (peer_service_name, await_connection),
            _ => panic!("Should never happen"),
        };

        trace!(
            "Trying to connect `{}` to the {}",
            requester_service_name,
            target_service_name
        );

        let (left, right) = UnixStream::pair().unwrap();

        {
            // No service file for the service
            if !self.permissions.service_file_exists(target_service_name) {
                warn!(
                    "`{}` wants to connect to `{}`, which doesn't exist",
                    requester_service_name, target_service_name
                );

                self.client(&requester_service_name)
                    .send_message(
                        &target_service_name,
                        BusError::ServiceNotFound.into_message(request.seq()),
                    )
                    .await;
                return;
            }

            if requester_service_name == *target_service_name {
                warn!(
                    "Service `{}` tries to connect to himself",
                    target_service_name,
                );

                self.client(&requester_service_name)
                    .send_message(
                        &target_service_name,
                        BusError::NotAllowed.into_message(request.seq()),
                    )
                    .await;
                return;
            }

            // Service to which our client wants to connect is not registered
            if !self.clients.contains_key(target_service_name) {
                // Peer doesn't want to wait for connection
                if !await_connection {
                    warn!(
                        "Failed to find a service `{}` to connect with `{}`",
                        target_service_name, requester_service_name
                    );

                    self.client(&requester_service_name)
                        .send_message(
                            &target_service_name,
                            BusError::ServiceNotRegisterd.into_message(request.seq()),
                        )
                        .await;
                    return;
                }

                // Peer wants to wait for a connection if service still not registered.
                // Add it to the pending list and return. Now client is sitting and waiting for
                // the response. See `handle_client_registration` for resolving code
                info!(
                    "Adding new pending connection from {} to {}",
                    requester_service_name, target_service_name
                );

                self.pending_connections
                    .entry(target_service_name.clone())
                    .or_insert(vec![])
                    .push(PendingConnectionRequest {
                        requester_service_name,
                        request,
                    });
                return;
            }

            // Send descriptor to the requester
            let message = ServiceMessage::IncomingPeerFd {
                peer_service_name: target_service_name.clone(),
            }
            .into_message(request.seq());
            self.client(&requester_service_name)
                .send_connection_fd(message, &target_service_name, left)
                .await;
        }

        // Send descriptor to the target service
        // NOTE: It's duplicates, but it's possible that hub will send different message
        let message = ServiceMessage::IncomingPeerFd {
            peer_service_name: requester_service_name.clone(),
        }
        .into_message(request.seq());
        self.client(&target_service_name)
            .send_connection_fd(message, &requester_service_name, right)
            .await;

        info!(
            "Succesfully connected `{}` to `{}`",
            requester_service_name, target_service_name
        )
    }

    /// Handle client disconnections
    async fn handle_client_disconnection(&mut self, uuid: &Uuid, service_name: &String) {
        self.anonymous_clients.remove(uuid);
        self.clients.remove(service_name);

        trace!("New named clients count: {}", self.clients.len());
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        info!("Shutting down Karo hub");

        if let Err(err) = fs::remove_file(common::get_hub_socket_path()) {
            error!("Failed to remove hub socket file: {}", err);
        }
    }
}
