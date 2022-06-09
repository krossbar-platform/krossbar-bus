use tokio::net::UnixStream;

pub struct Connection {
    connection_service_name: String,
    socket: UnixStream,
}

impl Connection {
    pub fn new(connection_service_name: String, socket: UnixStream) -> Self {
        Self {
            connection_service_name,
            socket,
        }
    }
}
