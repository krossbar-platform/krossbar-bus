use std::{error::Error, io::ErrorKind};

use log::*;
use tokio::sync::mpsc::{self, Sender};

use caro_bus_common::messages::Message;

pub(crate) type TaskResponse = Message;
pub(crate) type TaskCall = (Message, Sender<TaskResponse>);
pub(crate) type TaskChannel = Sender<TaskCall>;

/// Send message request into mpsc channel and wait for the result
pub async fn call_task(
    task_tx: &Sender<(Message, Sender<Message>)>,
    message: Message,
) -> Result<Message, Box<dyn Error + Send + Sync>> {
    let (tx, mut rx) = mpsc::channel(10);

    task_tx.send((message, tx)).await?;
    match rx.recv().await {
        Some(message) => Ok(message),
        None => {
            warn!("Failed to receive response from a task. Channel closed");
            Err(Box::new(std::io::Error::new(
                ErrorKind::BrokenPipe,
                "Channel closed",
            )))
        }
    }
}
