use std::{marker::PhantomData, sync::Arc};

use futures::{lock::Mutex, stream, Future, StreamExt};
use log::debug;
use serde::Serialize;

use krossbar_rpc::writer::RpcWriter;

type SubVectorType = Arc<Mutex<Vec<(i64, RpcWriter)>>>;

pub(crate) struct Handle {
    clients: SubVectorType,
}

impl Handle {
    fn new(clients: SubVectorType) -> Self {
        Self { clients }
    }

    pub(crate) async fn add_client(&self, sub_id: i64, writer: RpcWriter) {
        self.clients.lock().await.push((sub_id, writer))
    }
}

#[derive(Clone)]
pub struct Signal<T: Serialize> {
    clients: Arc<Mutex<Vec<(i64, RpcWriter)>>>,
    _marker: PhantomData<T>,
}

impl<T: Serialize> Signal<T> {
    pub(crate) fn new() -> Self {
        Self {
            clients: Arc::new(Mutex::new(Vec::new())),
            _marker: PhantomData,
        }
    }

    /// Emit the signal.
    /// Polling the result future broadcasts `data` to the subscribers.
    /// Method doesn't capture `self` into the result future, which allows moving the future into and async block.
    pub fn emit(&self, data: T) -> impl Future<Output = crate::Result<()>> {
        debug!("Emitting a signal");

        let clients = self.clients.clone();

        async move {
            let bson = match bson::to_bson(&data) {
                Ok(bson) => bson,
                Err(e) => return Err(crate::Error::ParamsTypeError(e.to_string())),
            };

            let mut client_lock = clients.lock().await;

            // Send data and remove clients, who don't want it anymore
            *client_lock = stream::iter(client_lock.drain(..))
                .filter_map(|(sub_id, client)| {
                    let data_copy = bson.clone();
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

    pub(crate) fn handle(&self) -> Handle {
        Handle::new(self.clients.clone())
    }
}
