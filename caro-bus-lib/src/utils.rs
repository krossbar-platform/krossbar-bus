use std::error::Error;

use tokio::sync::{
    mpsc::Sender,
    oneshot::{self, Sender as OneSender},
};

use caro_bus_common::messages::Message;

/// Send message request into mpsc channel and wait for the result
pub async fn call_task(
    task_tx: &Sender<(Message, OneSender<Message>)>,
    message: Message,
) -> Result<Message, Box<dyn Error + Send + Sync>> {
    let (one_tx, one_rx) = oneshot::channel();

    task_tx.send((message, one_tx)).await?;
    Ok(one_rx.await?)
}
