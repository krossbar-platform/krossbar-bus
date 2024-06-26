use std::{ops::Deref, sync::Arc};

use bson::Bson;
use futures::{lock::Mutex, stream, Future, StreamExt};
use log::debug;
use serde::Serialize;

use krossbar_rpc::writer::RpcWriter;

type SubVectorType = Arc<Mutex<Vec<(i64, RpcWriter)>>>;
type CurrentValueBson = Arc<Mutex<Bson>>;

pub(crate) struct Handle {
    clients: SubVectorType,
    current_value: CurrentValueBson,
}

impl Handle {
    fn new(clients: SubVectorType, current_value: CurrentValueBson) -> Self {
        Self {
            clients,
            current_value,
        }
    }

    pub(crate) async fn add_client(&self, sub_id: i64, writer: RpcWriter) {
        let response = Ok(self.current_value.lock().await.clone());

        if writer.respond(sub_id, response).await {
            self.clients.lock().await.push((sub_id, writer))
        }
    }

    pub(crate) async fn value(&self) -> Bson {
        self.current_value.lock().await.clone()
    }
}

pub struct State<T: Serialize> {
    clients: Arc<Mutex<Vec<(i64, RpcWriter)>>>,
    value: T,
    current_value: CurrentValueBson,
}

impl<T: Serialize> State<T> {
    pub(crate) fn new(value: T) -> crate::Result<Self> {
        let bson = match bson::to_bson(&value) {
            Ok(bson) => bson,
            Err(e) => return Err(crate::Error::ParamsTypeError(e.to_string())),
        };

        let current_value = Arc::new(Mutex::new(bson));

        Ok(Self {
            clients: Arc::new(Mutex::new(Vec::new())),
            value,
            current_value,
        })
    }

    /// Set the state to `value`.
    /// Polling the result future broadcasts `value` to the subscribers.
    /// Method doesn't capture `self` into the result future, which allows moving the future into and async block.
    pub fn set(&mut self, value: T) -> impl Future<Output = crate::Result<()>> {
        let value_bson = bson::to_bson(&value);

        debug!("Set state");

        self.value = value;

        let clients = self.clients.clone();
        let bson_value = self.current_value.clone();

        async move {
            if let Err(e) = value_bson {
                return Err(crate::Error::ParamsTypeError(e.to_string()));
            }

            let mut bson_lock = bson_value.lock().await;
            *bson_lock = value_bson.unwrap();

            let mut client_lock = clients.lock().await;

            // Send data and remove clients, who don't want it anymore
            *client_lock = stream::iter(client_lock.drain(..))
                .filter_map(|(sub_id, client)| {
                    let data_copy = bson_lock.clone();
                    async move {
                        if client.respond(sub_id, Ok(data_copy)).await {
                            Some((sub_id, client))
                        } else {
                            None
                        }
                    }
                })
                .collect()
                .await;

            Ok(())
        }
    }

    /// Get state value
    pub fn get(&self) -> &T {
        &self.value
    }

    pub(crate) fn handle(&self) -> Handle {
        Handle::new(self.clients.clone(), self.current_value.clone())
    }
}

impl<T: Serialize> Deref for State<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.get()
    }
}
