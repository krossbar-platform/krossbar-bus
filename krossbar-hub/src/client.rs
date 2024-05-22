use std::os::fd::AsRawFd;

use krossbar_bus_common::protocols::hub::{Message as HubMessage, HUB_CONNECT_METHOD};
use log::{info, warn};
use tokio::net::UnixStream;

use krossbar_common_rpc::{request::RpcRequest, rpc::Rpc, writer::RpcWriter, Error, Result};

use crate::hub::{ContextType, HubContext};

pub struct Client {
    context: ContextType,
    rpc: Rpc,
    service_name: String,
}

impl Client {
    pub fn new(context: ContextType, rpc: Rpc, service_name: String) -> Self {
        Self {
            context,
            rpc,
            service_name,
        }
    }

    pub async fn run(mut self) -> String {
        loop {
            match self.rpc.poll().await {
                Some(mut request) => {
                    if request.endpoint() != HUB_CONNECT_METHOD {
                        request
                            .respond::<()>(Err(Error::InternalError(format!(
                                "Expected only connection messages from a client. Got {} call",
                                request.endpoint()
                            ))))
                            .await;
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
                                    self.handle_connection_request(
                                        &self.service_name,
                                        &peer_service_name,
                                        request,
                                        wait,
                                    )
                                    .await;
                                }
                                // Connection request instead of an Auth message
                                Ok(m) => {
                                    warn!("Invalid connection message from a client: {m:?}");

                                    request
                                        .respond::<()>(Err(Error::InternalError(format!(
                                            "Invalid connection message body: {m:?}"
                                        ))))
                                        .await;
                                    return self.service_name;
                                }
                                // Message deserialization error
                                Err(e) => {
                                    warn!("Invalid connection message body from a client: {e:?}");

                                    request
                                        .respond::<()>(Err(Error::InternalError(e.to_string())))
                                        .await;
                                    return self.service_name;
                                }
                            }
                        }
                        // Not a call, but respond, of FD or other irrelevant message
                        _ => {
                            warn!("Invalid connection message from a client (not a call)");
                            return self.service_name;
                        }
                    }
                }
                _ => return self.service_name,
            }
        }
    }

    /// Send stream to services, which wait for the connected service
    pub async fn resolve_pending_connections(
        service_name: &str,
        stream: &mut RpcWriter,
        context: &mut HubContext,
    ) {
        if let Some(waiters) = context.pending_connections.remove(service_name) {
            for (initiator, request) in waiters.into_iter() {
                info!("Found service {initiator} which waits for a connected service {service_name}. Resolving now");

                // Send sockets
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
                let (fd1, fd2) = (socket1.as_raw_fd(), socket2.as_raw_fd());

                if let Err(e) = target_writer
                    .connection_request(initiator, target_service, socket1)
                    .await
                {
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

                let _ = nix::unistd::close(fd1);
                let _ = nix::unistd::close(fd2);

                info!("Succefully sent connection request from {initiator} to {target_service}");
            }
            Err(e) => {
                request
                    .respond::<()>(Err(Error::InternalError(e.to_string())))
                    .await;
            }
        }
    }

    async fn handle_connection_request(
        &self,
        service_name: &str,
        target_service: &str,
        request: RpcRequest,
        add_pending: bool,
    ) {
        info!(
            "Incoming connection request from {} to {}",
            service_name, service_name
        );

        let mut context_lock = self.context.lock().await;

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
                    if !context_lock
                        .permissions
                        .check_service_exists(target_service)
                    {
                        warn!(
                            "Failed to find a service which client wants to wait: {target_service}"
                        );

                        request.respond::<()>(Err(Error::ServiceNotFound)).await;
                    } else {
                        info!(
                            "Requested service {target_service} is down. Adding pending connection"
                        );

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
}
