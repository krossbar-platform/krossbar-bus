use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::Duration,
};

use async_recursion::async_recursion;
use bson::Bson;
use tokio::{net::UnixStream, sync::oneshot::Sender as OneSender};

use karo_bus_common::{
    self as common,
    connect::InspectData,
    errors::Error as BusError,
    messages::{Message, MessageBody, Response},
};

use crate::{method::Method, net::send_receive};

type Shared<T> = Arc<RwLock<T>>;
type MethodCall = (Bson, OneSender<Response>);

const RECONNECT_RETRY_PERIOD: Duration = Duration::from_secs(1);

pub struct Bus {
    service_name: Arc<String>,
    /// Registered methods. Sender is used to send parameters and
    /// receive a result from a callback
    methods: Shared<HashMap<String, Box<dyn Method>>>,
    /// Data for service inspection
    inspect_data: Shared<InspectData>,
}

impl Bus {
    pub async fn start(service_name: &str) -> crate::Result<Self> {
        let this = Self {
            service_name: Arc::new(service_name.into()),
            methods: Arc::new(RwLock::new(HashMap::new())),
            inspect_data: Arc::new(RwLock::new(InspectData::new())),
        };

        let socket = UnixStream::connect(common::get_hub_socket_path()).await?;
        this.start_loop(service_name, socket).await?;

        Ok(this)
    }

    async fn start_loop(&self, service_name: &str, mut socket: UnixStream) -> crate::Result<()> {
        // Make a message and send to the hub
        let message = Message::new_registration(service_name.into());

        Self::register(service_name.into(), &mut socket);

        Ok(())
    }

    #[async_recursion]
    async fn reconnect(service_name: String) -> UnixStream {
        loop {
            let service_name_clone = service_name.clone();

            // Try to reconnect to the hub. I fails, we just retry
            match UnixStream::connect(common::get_hub_socket_path()).await {
                Ok(mut socket) => {
                    println!("Succesfully reconnected to the hub");

                    // Try to connect and register
                    match Self::register(service_name_clone, &mut socket).await {
                        Ok(_) => return socket,
                        Err(err) => {
                            eprintln!(
                                "Failed to reconnect service `{}`: {}",
                                service_name,
                                err.to_string()
                            );

                            tokio::time::sleep(RECONNECT_RETRY_PERIOD).await;
                            continue;
                        }
                    }
                }
                Err(_) => {
                    println!("Failed to reconnect to a hub socket. Will try again");

                    tokio::time::sleep(RECONNECT_RETRY_PERIOD).await;
                    continue;
                }
            }
        }
    }

    /// Send registration request
    /// This method is a blocking call from the library workflow perspectire: we can't make any hub
    /// calls without previous registration
    async fn register(service_name: String, socket: &mut UnixStream) -> crate::Result<()> {
        // First connect to a socket
        println!("Performing service `{}` registration", service_name);

        loop {
            // Make a message and send to the hub
            let message = Message::new_registration(service_name.clone());

            match send_receive(message, socket).await {
                Some(reponse) => match reponse.body() {
                    MessageBody::Response(Response::Ok) => {
                        println!("Succesfully registered service as `{}`", service_name);

                        return Ok(());
                    }
                    MessageBody::Response(Response::Error(err)) => {
                        eprintln!("Failed to register service as `{}`: {}", service_name, err);
                        return Err(Box::new(err.clone()));
                    }
                    m => {
                        eprintln!("Invalid response from the hub: {:?}", m);
                        return Err(Box::new(BusError::InvalidMessage));
                    }
                },
                None => {
                    *socket = Self::reconnect(service_name.clone()).await;
                }
            }
        }
    }
}
