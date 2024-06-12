use std::{
    collections::HashMap, fs, os::unix::fs::PermissionsExt, path::PathBuf, pin::Pin, sync::Arc,
};

use futures::{
    future::{pending, FutureExt as _},
    lock::Mutex,
    select,
    stream::FuturesUnordered,
    Future, StreamExt as _,
};
use krossbar_state_machine::Machine;
use log::{debug, info, warn};
use tokio::net::{unix::UCred, UnixListener};

use krossbar_bus_common::protocols::hub::{Message as HubMessage, HUB_REGISTER_METHOD};
use krossbar_rpc::{request::RpcRequest, rpc::Rpc, writer::RpcWriter, Error, Result};

use crate::{args::Args, client::Client, permissions::Permissions};

type TasksMapType = FuturesUnordered<Pin<Box<dyn Future<Output = Option<String>> + Send>>>;
pub type ContextType = Arc<Mutex<HubContext>>;

pub struct HubContext {
    pub client_registry: HashMap<String, RpcWriter>,
    pub pending_connections: HashMap<String, Vec<(String, RpcRequest)>>,
    pub permissions: Permissions,
}

pub struct Hub {
    tasks: TasksMapType,
    socket_path: PathBuf,
    context: ContextType,
}

impl Hub {
    pub fn new(args: Args) -> Self {
        let tasks: TasksMapType = FuturesUnordered::new();
        tasks.push(Box::pin(pending()));

        Self {
            tasks,
            socket_path: args.socket_path.clone(),
            context: Arc::new(Mutex::new(HubContext {
                client_registry: HashMap::new(),
                pending_connections: HashMap::new(),
                permissions: Permissions::new(&args.additional_service_dirs),
            })),
        }
    }

    /// Hub main loop
    pub async fn run(mut self) {
        info!("Hub socket path: {:?}", self.socket_path);

        let listener = match UnixListener::bind(&self.socket_path) {
            Ok(listener) => listener,
            Err(e) => {
                warn!("Failed to start listening: {e:?}. Trying to remove hanging socket");

                let _ = std::fs::remove_file(&self.socket_path);
                UnixListener::bind(&self.socket_path).unwrap()
            }
        };

        info!("Hub started listening for new connections");

        // Update permissions to be accessible for th eclient
        let socket_permissions = fs::Permissions::from_mode(0o666);
        fs::set_permissions(&self.socket_path, socket_permissions).unwrap();

        async move {
            loop {
                select! {
                    // Accept new connection requests
                    client = listener.accept().fuse() => {
                        match client {
                            Ok((stream, _)) => {
                                let credentials = stream.peer_cred();
                                let rpc = Rpc::new(stream, "");

                                match credentials {
                                    Ok(credentials) => {
                                        info!("New connection request: {credentials:?}");

                                        let client_machine = Machine::init((rpc, credentials, self.context.clone()))
                                            .then(Self::authorize)
                                            .then(Client::run)
                                            .unwrap(Self::client_name);

                                        self.tasks.push(Box::pin(client_machine))
                                    },
                                    Err(e) => {
                                        warn!("Failed to get client creadentials: {}", e.to_string());
                                    }
                                }

                            },
                            Err(e) => {
                                warn!("Failed client connection attempt: {}", e.to_string())
                            }
                        }
                    },
                    // Loop clients. Loop return means a client is disconnected
                    disconnected_service = self.tasks.next() => {
                        let service_name = disconnected_service.unwrap();

                        match service_name {
                            Some(service_name) => {
                                debug!("Client disconnected: {}", service_name);
                                self.context.lock().await.client_registry.remove(&service_name);
                            }
                            _ => {
                                debug!("Anonymous client disconnected");
                            }
                        }
                    },
                    _ = tokio::signal::ctrl_c().fuse() => return
                }
            }
        }
        .await;

        // Cleanup socket
        let _ = std::fs::remove_file(&self.socket_path);
    }

    async fn authorize(
        (mut rpc, credentials, context): (Rpc, UCred, ContextType),
    ) -> std::result::Result<(Rpc, ContextType, String), ()> {
        debug!("New client connection. Waiting for an auth message");

        // Authorize the client
        let service_name = match rpc.poll().await {
            Some(mut request) => {
                if request.endpoint() != HUB_REGISTER_METHOD {
                    request
                        .respond::<()>(Err(Error::InternalError(format!(
                            "Expected registration call from a client. Got {}",
                            request.endpoint()
                        ))))
                        .await;
                }

                match request.take_body().unwrap() {
                    // Valid call message
                    krossbar_rpc::request::Body::Call(bson) => {
                        // Valid Auth message
                        match bson::from_bson::<HubMessage>(bson) {
                            Ok(HubMessage::Register { service_name }) => {
                                // Check permissions
                                match Self::handle_auth_request(
                                    &service_name,
                                    &request,
                                    credentials,
                                    &context,
                                )
                                .await
                                {
                                    Ok(_) => {
                                        info!("Succesfully authorized {service_name}");
                                        request.respond(Ok(())).await;

                                        let mut writer = request.writer().clone();
                                        Client::resolve_pending_connections(
                                            &service_name,
                                            &mut writer,
                                            &context,
                                        )
                                        .await;

                                        service_name
                                    }
                                    Err(e) => {
                                        warn!("Service {service_name} is not allowed to register");
                                        request.respond::<()>(Err(e)).await;
                                        request.writer().flush().await;

                                        return Err(());
                                    }
                                }
                            }
                            // Connection request instead of an Auth message
                            Ok(m) => {
                                warn!("Invalid registration message from a client: {m:?}");

                                request
                                    .respond::<()>(Err(Error::InternalError(format!(
                                        "Invalid register message body: {m:?}"
                                    ))))
                                    .await;
                                request.writer().flush().await;

                                return Err(());
                            }
                            // Message deserialization error
                            Err(e) => {
                                warn!("Invalid Auth message body from a client: {e:?}");

                                request
                                    .respond::<()>(Err(Error::InternalError(e.to_string())))
                                    .await;
                                request.writer().flush().await;

                                return Err(());
                            }
                        }
                    }
                    // Not a call, but respond, of FD or other irrelevant message
                    _ => {
                        warn!("Invalid Auth message from a client (not a call)");
                        request
                            .respond::<()>(Err(Error::InternalError(
                                "Waiting for a registration message. Received a call".to_owned(),
                            )))
                            .await;
                        request.writer().flush().await;

                        return Err(());
                    }
                }
            }
            // Client disconnected
            _ => {
                return Err(());
            }
        };

        Ok((rpc, context, service_name))
    }

    fn client_name(status: std::result::Result<String, ()>) -> Option<String> {
        match status {
            Ok(service_name) => Some(service_name),
            _ => None,
        }
    }

    /// Handle client Auth message
    async fn handle_auth_request(
        service_name: &str,
        request: &RpcRequest,
        credentials: UCred,
        context: &ContextType,
    ) -> Result<()> {
        debug!("Service registration request: {}", service_name);

        let mut context_lock = context.lock().await;

        // Check if we already have a client with the name
        if context_lock.client_registry.contains_key(service_name) {
            warn!(
                "Multiple service registration request from: {}",
                service_name
            );

            Err(Error::AlreadyRegistered)
        // The only valid Auth request path
        } else {
            if !context_lock
                .permissions
                .check_service_name_allowed(credentials, service_name)
            {
                debug!("Client {service_name} is not allowed to register with a given credentials");

                return Err(Error::NotAllowed);
            }

            context_lock
                .client_registry
                .insert(service_name.to_owned(), request.writer().clone());

            info!("Client authorized as: {}", service_name);

            Ok(())
        }
    }
}
