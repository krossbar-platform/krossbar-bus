use std::error::Error;

use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use common::messages::{self, Message};

pub async fn send(socket: &mut UnixStream, message: Message) -> Result<(), Box<dyn Error>> {
    let data = message.bytes();

    match socket.write_all(&data).await {
        Ok(_) => Ok(()),
        Err(err) => Err(Box::new(err)),
    }
}

pub async fn send_receive(
    socket: &mut UnixStream,
    message: Message,
) -> Result<Message, Box<dyn Error>> {
    send(socket, message).await?;

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
