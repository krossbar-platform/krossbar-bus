use std::io::{ErrorKind, Result as IoResult};

use bytes::BytesMut;
use log::*;
use tokio::{io::AsyncReadExt, net::UnixStream};

use crate::messages::{self, EitherMessage, Message};

/// Function to read Message from a socket. This function is intended to read exact amount
/// of data to parse a message. The lib need this, because we may have a file descriptor right after a
/// message. And we want to keep those descriptor to read it with passfd::recv_fd
pub async fn read_message(socket: &mut UnixStream, buffer: &mut BytesMut) -> IoResult<Message> {
    // First read Bson length
    let mut bytes_to_read = 4;

    loop {
        // Make a handle to read exact amount of data
        let mut take_handle = socket.take(bytes_to_read);

        match take_handle.read_buf(buffer).await {
            Ok(bytes_read) => {
                // Socket closed
                if bytes_read == 0 {
                    trace!("Read zero bytes from a socket");

                    return Err(ErrorKind::BrokenPipe.into());
                }

                // Descrease bytes by number of bytes already read
                bytes_to_read = bytes_to_read - bytes_read as u64;
                trace!(
                    "Read {} bytes from socket. Still {} to read",
                    bytes_read,
                    bytes_to_read
                );

                // Still need more data to read
                if bytes_to_read != 0 {
                    continue;
                }

                // Try to parse message to take exact amount of data we need to read to get a message
                match messages::parse_buffer(buffer) {
                    EitherMessage::FullMessage(message) => return Ok(message),
                    EitherMessage::NeedMoreData(len) => {
                        trace!("Parser asks for {} more bytes to read", len);
                        // Try to read exact amount of data to get a message next time
                        bytes_to_read = len as u64;
                        continue;
                    }
                }
            }
            Err(err) => {
                error!(
                    "Failed to read from a socket: {}. Client is disconnected. Shutting him down",
                    err.to_string()
                );
                return Err(err);
            }
        };
    }
}
