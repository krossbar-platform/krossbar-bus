use std::{
    collections::HashMap,
    io::Result as IoResult,
    sync::{
        atomic::{AtomicU64, Ordering},
        {Arc, RwLock},
    },
};

use log::*;
use tokio::io::AsyncWriteExt;
use tokio::{net::UnixStream, sync::mpsc::Sender};

use crate::messages::MessageBody;

use super::messages::Message;

type Shared<T> = Arc<RwLock<T>>;

#[derive(Clone)]
struct Call {
    persistent: bool,
    callback: Sender<Message>,
}

/// Struct to account user calls and send incoming response
/// to a proper caller.
/// Registry keep subscription across [CallRegistry::resolve] calls, but deletes
/// one time calls
#[derive(Clone)]
pub struct CallRegistry {
    seq_counter: Arc<AtomicU64>,
    calls: Shared<HashMap<u64, Call>>,
}

impl CallRegistry {
    pub fn new() -> Self {
        Self {
            seq_counter: Arc::new(0.into()),
            calls: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Make a call through the *socket*. Message seq_no will be replaced with
    /// a proper one.
    /// *Returns* oneshot receiver client can await for to get call response
    pub async fn call(
        &mut self,
        socket: &mut UnixStream,
        message: &mut Message,
        callback: &Sender<Message>,
    ) -> IoResult<()> {
        let seq = self.seq_counter.fetch_add(1, Ordering::SeqCst);
        message.seq = seq;

        let persistent = CallRegistry::is_persistent_call(&message);
        trace!(
            "Registerng a call with seq {}: {:?}. {}",
            message.seq(),
            message.body(),
            if persistent { "Persistent" } else { "One time" }
        );

        socket.write_all(message.bytes().as_slice()).await?;

        self.calls.write().unwrap().insert(
            seq,
            Call {
                persistent,
                callback: callback.clone(),
            },
        );
        Ok(())
    }

    /// Resolve a client call with a given *message*
    pub async fn resolve(&mut self, message: Message) {
        // Keep subscriptions
        let seq = message.seq();

        let maybe_sender = self.calls.read().unwrap().get(&message.seq).cloned();

        match maybe_sender {
            Some(call) => {
                trace!("Succesfully resolved a call {}", message.seq);

                if let Err(mess) = call.callback.send(message).await {
                    error!("Failed to send response to a client: {:?}", mess);
                    return;
                }

                if !call.persistent {
                    trace!("Removing resolved call: {}", seq);
                    self.calls.write().unwrap().remove(&seq);
                }
            }
            _ => {
                warn!("Unknown client response: {:?}", message);
            }
        }
    }

    pub fn has_call(&self, seq: u64) -> bool {
        self.calls.read().unwrap().contains_key(&seq)
    }

    /// If we should keep sender after resolving. Used for subscriptions
    fn is_persistent_call(message: &Message) -> bool {
        matches!(message.body(), MessageBody::SignalSubscription { .. })
            || matches!(message.body(), MessageBody::StateSubscription { .. })
    }
}
