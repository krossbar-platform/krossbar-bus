use std::error::Error;

use async_trait::async_trait;
use bson::Bson;
use log::*;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::oneshot::{self, Sender as OneSender};

#[async_trait]
pub trait MethodTrait: Send + Sync {
    async fn notify(&self, parameters: Bson) -> Result<Bson, Box<dyn Error>>;
}

pub struct MethodCall<P: Send + Sync + DeserializeOwned, R: Send + Sync + Serialize> {
    params: P,
    reply: OneSender<R>,
}

impl<P: Send + Sync + DeserializeOwned, R: Send + Sync + Serialize> MethodCall<P, R> {
    pub fn params(&self) -> &P {
        &self.params
    }

    pub async fn reply(self, response: R) {
        if let Err(_) = self.reply.send(response) {
            error!("Failed to send method call reply");
        }
    }
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
    async fn notify(&self, parameters: Bson) -> Result<Bson, Box<dyn Error>> {
        let (reply, rx) = oneshot::channel();

        match bson::from_bson::<P>(parameters) {
            Ok(params) => {
                let _ = self.callback_send.send(MethodCall { params, reply }).await;

                let response = rx.await.map_err(|err| err.to_string())?;
                bson::to_bson(&response).map_err(|e| e.into())
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
