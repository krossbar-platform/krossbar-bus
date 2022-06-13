use std::error::Error;

use async_trait::async_trait;
use bson::Bson;
use log::*;
use serde::{de::DeserializeOwned, Serialize};
use tokio::sync::{
    mpsc::{self, Receiver, Sender},
    oneshot::{self, Sender as OneSender},
};

/// A trait to be stored into a map
#[async_trait]
pub trait MethodTrait: Send + Sync {
    async fn notify(&self, parameters: Bson) -> Result<Bson, Box<dyn Error>>;
}

/// Method call handle passed to a client
pub struct MethodCall<P: Send + Sync + DeserializeOwned, R: Send + Sync + Serialize> {
    /// Incoming parameters from a peer
    params: P,
    /// Reply tx
    reply: OneSender<R>,
}

impl<P: Send + Sync + DeserializeOwned, R: Send + Sync + Serialize> MethodCall<P, R> {
    /// Get method call parameters
    pub fn params(&self) -> &P {
        &self.params
    }

    /// Reply to the peer
    pub async fn reply(self, response: R) {
        if let Err(_) = self.reply.send(response) {
            error!("Failed to send method call reply");
        }
    }
}

/// Generic method handle, which keeps type info to perform proper conversions
/// between wired data on the onr side and method parameters and result on the other
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
    /// Calls user collback for the method.
    /// * `parameters` - Method parameters from the peer. Will be converted into type `P` to
    ///     pass to the user
    /// Returns serialized response from the user
    async fn notify(&self, parameters: Bson) -> Result<Bson, Box<dyn Error>> {
        let (reply, rx) = oneshot::channel();

        // First try to deserialize method peer parameter into type `P` registered by the lib user.
        // If we've failed to serialize, it means that our peer makes a call using invalid paramters type
        match bson::from_bson::<P>(parameters) {
            Ok(params) => {
                // Notify the user about the method call
                let _ = self.callback_send.send(MethodCall { params, reply }).await;

                // Receive method call response
                let response = rx.await.map_err(|err| err.to_string())?;
                // Try to deserialize user's response into binary format
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
