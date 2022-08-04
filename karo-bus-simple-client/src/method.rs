use async_trait::async_trait;
use bson::Bson;
use serde::de::DeserializeOwned;
use tokio::sync::mpsc::{self, Receiver, Sender};

#[async_trait]
pub(crate) trait Method {
    async fn call(&self, data: Bson);
}

pub(crate) struct TypedMethod<T> {
    tx: Sender<T>,
}

impl<T> TypedMethod<T> {
    pub fn new() -> (Self, Receiver<T>) {
        let (tx, rx) = mpsc::channel(32);

        let this = Self { tx };
        (this, rx)
    }
}

#[async_trait]
impl<T: DeserializeOwned + Send> Method for TypedMethod<T> {
    async fn call(&self, data: Bson) {
        match bson::from_bson::<T>(data) {
            Ok(params) => {
                if let Err(err) = self.tx.send(params).await {
                    eprintln!("Failed to send method call: {}", err.to_string());
                }
            }
            Err(err) => {
                eprintln!(
                    "Failed to deserialize method call parameters: {}",
                    err.to_string()
                );
            }
        }
    }
}
