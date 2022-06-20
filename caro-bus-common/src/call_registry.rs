use std::{
    collections::HashMap,
    io::Result as IoResult,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use log::*;
use parking_lot::RwLock;
use tokio::io::AsyncWriteExt;
use tokio::{net::UnixStream, sync::oneshot::Sender as OneSender};

use super::messages::Message;

type Shared<T> = Arc<RwLock<T>>;

/// Struct to account user calls and send incoming response
/// to a proper caller
#[derive(Clone)]
pub struct CallRegistry {
    seq_counter: Arc<AtomicU64>,
    calls: Shared<HashMap<u64, OneSender<Message>>>,
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
        mut message: Message,
        callback: OneSender<Message>,
    ) -> IoResult<()> {
        let seq = self.seq_counter.fetch_add(1, Ordering::SeqCst);
        message.seq = seq;

        trace!(
            "Registerng a call with seq {}: {:?}",
            message.seq(),
            message
        );

        socket.write_all(message.bytes().as_slice()).await?;

        self.calls.write().insert(seq, callback);
        Ok(())
    }

    /// Resolve a client call with a given *message*
    pub fn resolve(&mut self, message: Message) {
        let mut calls = self.calls.write();

        match calls.remove(&message.seq) {
            Some(sender) => {
                trace!("Succesfully resolved a call {}", message.seq);

                if let Err(mess) = sender.send(message) {
                    error!("Failed to send response to a client: {:?}", mess);
                }
            }
            _ => {
                warn!("Unknown client response: {:?}", message);
            }
        }
    }

    pub fn has_call(&self, seq: u64) -> bool {
        self.calls.read().contains_key(&seq)
    }
}
