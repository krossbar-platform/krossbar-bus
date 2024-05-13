use std::{collections::HashMap, fs, os::unix::fs::PermissionsExt, pin::Pin, sync::Arc};

use futures::{
    future::{pending, FutureExt as _},
    lock::Mutex,
    select,
    stream::FuturesUnordered,
    Future, StreamExt as _,
};
use log::{debug, info, warn};
use tokio::net::{unix::UCred, UnixListener, UnixStream};

use krossbar_bus_common::{
    get_hub_socket_path, message::HubMessage, HUB_CONNECT_METHOD, HUB_REGISTER_METHOD,
};
use krossbar_common_rpc::{request::RpcRequest, rpc::Rpc, writer::RpcWriter, Error, Result};

use crate::{args::Args, permissions::Permissions};

type TasksMapType = FuturesUnordered<Pin<Box<dyn Future<Output = Option<String>> + Send>>>;
type ContextType = Arc<Mutex<HubContext>>;

struct HubContext {
    pub client_registry: HashMap<String, RpcWriter>,
    pub pending_connections: HashMap<String, Vec<(String, RpcRequest)>>,
    pub permissions: Permissions,
}

pub struct Hub {
    tasks: TasksMapType,
    context: ContextType,
}

impl Hub {
    pub fn new(args: Args) -> Self {
        let tasks: TasksMapType = FuturesUnordered::new();
        tasks.push(Box::pin(pending()));

        Self {
            tasks,
            context: Arc::new(Mutex::new(HubContext {
                client_registry: HashMap::new(),
                pending_connections: HashMap::new(),
                permissions: Permissions::new(&args.additional_service_dirs),
            })),
        }
    }

    /// Hub main loop
    pub async fn run(mut self) {
        let socket_addr = get_hub_socket_path();
        info!("Hub socket path: {socket_addr}");

        let listener = match UnixListener::bind(&socket_addr) {
            Ok(listener) => listener,
            Err(_) => {
                let _ = std::fs::remove_file(&socket_addr);
                let result = UnixListener::bind(socket_addr).unwrap();

                info!("Hub started listening for new connections");
                result
            }
        };

        // Update permissions to be accessible for th eclient
        let socket_permissions = fs::Permissions::from_mode(0o666);
        fs::set_permissions(get_hub_socket_path(), socket_permissions).unwrap();

        async move {
            loop {
                select! {
                    // Accept new connection requests
                    client = listener.accept().fuse() => {
                        match client {
                            Ok((stream, _)) => {
                                let credentials = stream.peer_cred();
                                let rpc = Rpc::new(stream);

                                match credentials {
                                    Ok(credentials) => {
                                        info!("New connection request: {credentials:?}");
                                        let connection = Self::make_new_connection(rpc, credentials, self.context.clone());

                                        self.tasks.push(Box::pin(connection))
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
    }

    /// Make a connection form a stream
    async fn make_new_connection(
        mut rpc: Rpc,
        credentials: UCred,
        context: ContextType,
    ) -> Option<String> {
        // Authorize the client
        let service_name = match rpc.poll().await {
            Some(mut request) => {
                if request.endpoint() != HUB_REGISTER_METHOD {
                    request.respond::<()>(Err(Error::ProtocolError)).await;
                }

                match request.take_body().unwrap() {
                    // Valid call message
                    krossbar_common_rpc::request::Body::Call(bson) => {
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

                                        service_name
                                    }
                                    Err(e) => {
                                        warn!("Service {service_name} is not allowed to register");
                                        request.respond::<()>(Err(e)).await;
                                        return None;
                                    }
                                }
                            }
                            // Connection request instead of an Auth message
                            Ok(m) => {
                                warn!("Invalid registration message from a client: {m:?}");

                                request.respond::<()>(Err(Error::ProtocolError)).await;
                                return None;
                            }
                            // Message deserialization error
                            Err(e) => {
                                warn!("Invalid Auth message body from a client: {e:?}");

                                request
                                    .respond::<()>(Err(Error::InternalError(e.to_string())))
                                    .await;
                                return None;
                            }
                        }
                    }
                    // Not a call, but respond, of FD or other irrelevant message
                    _ => {
                        warn!("Invalid Auth message from a client (not a call)");
                        return None;
                    }
                }
            }
            // Client disconnected
            _ => {
                return None;
            }
        };

        // Cient succesfully authorized. Start client loop
        Some(Self::client_loop(rpc, service_name, context.clone()).await)
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

            return Err(Error::AlreadyRegistered);
        // The only valid Auth request path
        } else {
            if !context_lock
                .permissions
                .check_service_name_allowed(credentials, service_name)
            {
                debug!("Client {service_name} is not allowed to register with a given credentials");

                return Err(Error::NotAllowed);
            }

            let mut writer = request.writer().clone();
            Self::resolve_pending_connections(service_name, &mut writer, &mut context_lock).await;

            context_lock
                .client_registry
                .insert(service_name.to_owned(), writer);

            info!("Client authorized as: {}", service_name);

            Ok(())
        }
    }

    /// Send stream to services, which wait for the connected service
    async fn resolve_pending_connections(
        service_name: &str,
        stream: &mut RpcWriter,
        context: &mut HubContext,
    ) {
        if let Some(waiters) = context.pending_connections.remove(service_name) {
            for (initiator, request) in waiters.into_iter() {
                // Let's check if we have alive initiator
                Self::send_connection_descriptors(&initiator, request, service_name, stream).await
            }
        }
    }

    async fn send_connection_descriptors(
        initiator: &str,
        request: RpcRequest,
        target_service: &str,
        target_writer: &RpcWriter,
    ) {
        match UnixStream::pair() {
            Ok((socket1, socket2)) => {
                if let Err(e) = target_writer.connection_request(&initiator, socket1).await {
                    warn!(
                        "Failed to send target connection request: {}",
                        e.to_string()
                    );

                    request
                        .respond::<Result<()>>(Err(Error::PeerDisconnected))
                        .await;
                } else {
                    request.respond_with_fd(Ok(()), socket2).await;
                }

                info!("Succefully sent connection request from {initiator} to {target_service}");
            }
            Err(e) => {
                request
                    .respond::<()>(Err(Error::InternalError(e.to_string())))
                    .await;
            }
        }
    }

    async fn client_loop(mut rpc: Rpc, service_name: String, context: ContextType) -> String {
        loop {
            match rpc.poll().await {
                Some(mut request) => {
                    if request.endpoint() != HUB_CONNECT_METHOD {
                        request.respond::<()>(Err(Error::ProtocolError)).await;
                    }

                    match request.take_body().unwrap() {
                        // Valid call message
                        krossbar_common_rpc::request::Body::Call(bson) => {
                            // Valid Auth message
                            match bson::from_bson::<HubMessage>(bson) {
                                Ok(HubMessage::Connect {
                                    service_name: peer_service_name,
                                    wait,
                                }) => {
                                    Self::handle_connection_request(
                                        &service_name,
                                        &peer_service_name,
                                        request,
                                        wait,
                                        &context,
                                    )
                                    .await;
                                }
                                // Connection request instead of an Auth message
                                Ok(m) => {
                                    warn!("Invalid connection message from a client: {m:?}");

                                    request.respond::<()>(Err(Error::ProtocolError)).await;
                                    return service_name;
                                }
                                // Message deserialization error
                                Err(e) => {
                                    warn!("Invalid connection message body from a client: {e:?}");

                                    request
                                        .respond::<()>(Err(Error::InternalError(e.to_string())))
                                        .await;
                                    return service_name;
                                }
                            }
                        }
                        // Not a call, but respond, of FD or other irrelevant message
                        _ => {
                            warn!("Invalid connection message from a client (not a call)");
                            return service_name;
                        }
                    }
                }
                _ => return service_name,
            }
        }
    }

    async fn handle_connection_request(
        service_name: &str,
        target_service: &str,
        request: RpcRequest,
        add_pending: bool,
        context: &ContextType,
    ) {
        info!(
            "Incoming connection request from {} to {}",
            service_name, service_name
        );

        let mut context_lock = context.lock().await;

        // Check if service allowed to connect
        if !context_lock
            .permissions
            .check_connection_allowed(service_name, target_service)
        {
            request.respond::<()>(Err(Error::NotAllowed)).await;
            return;
        }

        match context_lock.client_registry.get(target_service) {
            Some(target_writer) => {
                Self::send_connection_descriptors(
                    service_name,
                    request,
                    target_service,
                    target_writer,
                )
                .await
            }
            _ => {
                if !add_pending {
                    request.respond::<()>(Err(Error::ServiceNotFound)).await;
                } else {
                    context_lock
                        .pending_connections
                        .entry(target_service.to_owned())
                        .or_default()
                        .push((service_name.to_owned(), request));
                }
            }
        }
    }
}
