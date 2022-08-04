use bytes::BytesMut;
use tokio::{io::AsyncWriteExt, net::UnixStream};

use karo_bus_common::{messages::Message, net};

pub async fn send_receive(message: Message, socket: &mut UnixStream) -> Option<Message> {
    if let Err(err) = socket.write_all(message.bytes().as_slice()).await {
        eprintln!(
            "Failed to send write a message to the peer: {}. Reconnecting",
            err.to_string()
        );

        return None;
    }

    let mut bytes = BytesMut::with_capacity(64);

    match net::read_message_from_socket(socket, &mut bytes).await {
        Ok(message) => Some(message),
        Err(err) => {
            eprintln!(
                "Failed to read message from a peer: {}. Reconnecting",
                err.to_string()
            );
            None
        }
    }
}
