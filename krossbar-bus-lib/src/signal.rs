use async_channel::{unbounded, Receiver, Sender};
use log::trace;

#[derive(Clone)]
pub(crate) struct AsyncSignal<T: Send> {
    sender: Sender<T>,
    receiver: Receiver<T>,
}

impl<T: Send> AsyncSignal<T> {
    pub fn new() -> Self {
        let (sender, receiver) = unbounded();
        Self { sender, receiver }
    }

    pub async fn emit(&self, value: T) {
        trace!("Signal emit");
        self.sender.send(value).await.unwrap()
    }

    pub async fn wait(&self) -> T {
        trace!("Signal await");
        self.receiver.recv().await.unwrap()
    }
}
