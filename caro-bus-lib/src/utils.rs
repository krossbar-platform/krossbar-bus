use std::error::Error;

use bytes::BytesMut;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
    sync::{
        mpsc::Sender,
        oneshot::{self, Sender as OneSender},
    },
};

use caro_bus_common::messages::{self, Message};

/// Send message into a socket
pub async fn send_message(
    socket: &mut UnixStream,
    message: Message,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let data = message.bytes();

    match socket.write_all(&data).await {
        Ok(_) => Ok(()),
        Err(err) => Err(Box::new(err)),
    }
}

/// Send message into a socket and wait for the result
pub async fn send_receive_message(
    socket: &mut UnixStream,
    message: Message,
) -> Result<Message, Box<dyn Error + Send + Sync>> {
    send_message(socket, message).await?;

    let mut bytes = BytesMut::with_capacity(64);
    loop {
        match socket.read_buf(&mut bytes).await {
            Ok(_len) => {
                if let Some(message) = messages::parse_buffer(&mut bytes) {
                    return Ok(message);
                }
            }
            Err(error) => return Err(Box::new(error)),
        }
    }
}

/// Send message request into mpsc channel and wait for the result
pub async fn call_task(
    task_tx: &Sender<(
        Message,
        OneSender<Result<Message, Box<dyn Error + Send + Sync>>>,
    )>,
    message: Message,
) -> Result<Message, Box<dyn Error + Send + Sync>> {
    let (one_tx, one_rx) = oneshot::channel();

    task_tx.send((message, one_tx)).await?;
    one_rx.await?
}
