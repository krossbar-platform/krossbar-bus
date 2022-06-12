use std::error::Error;

use async_trait::async_trait;
use bson::{self, raw::RawDocumentBuf, Bson};
use log::*;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::oneshot::{self, Sender as OneSender};

#[async_trait]
pub trait MethodTrait: Send + Sync {
    async fn notify(&mut self, parameters: Bson) -> Result<RawDocumentBuf, Box<dyn Error>>;
}

pub struct MethodCall<P: Send + Sync + DeserializeOwned, R: Send + Sync + Serialize> {
    params: P,
    reply: OneSender<R>,
}

pub struct Method<P: Send + Sync + DeserializeOwned, R: Send + Sync + Serialize> {
    callback_send: Sender<MethodCall<P, R>>,
}

impl<P: Send + Sync + DeserializeOwned, R: Send + Sync + Serialize> Method<P, R> {
    pub fn new() -> (Self, Receiver<MethodCall<P, R>>) {
        let (callback_send, rx) = mpsc::channel(32);

        (Self { callback_send }, rx)
    }
}

#[async_trait]
impl<P: Send + Sync + DeserializeOwned, R: Send + Sync + Serialize> MethodTrait for Method<P, R> {
    async fn notify(&mut self, parameters: Bson) -> Result<RawDocumentBuf, Box<dyn Error>> {
        let (reply, rx) = oneshot::channel();

        match bson::from_bson::<P>(parameters) {
            Ok(params) => {
                let _ = self.callback_send.send(MethodCall { params, reply }).await;

                let response = rx.await.map_err(|err| err.to_string())?;

                Ok(bson::to_raw_document_buf(&response).unwrap())
            }
            Err(err) => {
                warn!(
                    "Failed to deserialize method call parameters: {}",
                    err.to_string()
                );

                Err(Box::new(err))
            }
        }
    }
}
