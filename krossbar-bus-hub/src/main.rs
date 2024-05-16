//! ## Krossbar bus hub
//!
//! The binary acts as a hub for service connections. On request it makes a UDS pair and sends corresponding sockets
//! to each of the peers, which afterward use the pair for communication.
//! The only known point of rendezvous fot the services is the hub socket.
//!
//! The hub uses [krossbar_bus_common::DEFAULT_HUB_SOCKET_PATH] for hub socket path and
//! [krossbar_bus_common::DEFAULT_SERVICE_FILES_DIR] for service files dir by default.
//! These can be changed using cmd args.
//!
//! Created socket has 0o666 file permission to allow service connections.
//!
//! ## Service files
//! During service registration and later for all connection requests the hub uses permission system to check if the service
//! is allowed to do what it's trying to do.
//!
//! Service file filename identifies client service name. The file itself contains executable glob for which it's allowed
//! to register the service, and a list of client names, who are allowed to connect to the service.
//!
//! Basic service file `com.example.echo.service` may look like the following:
//! ```json
//! {
//! "exec": "/data/krossbar/*",
//! "incoming_connections": ["**"]
//! }
//! ```
//!
//! See [lib examples](https://github.com/krossbar-platform/krossbar-bus/tree/main/krossbar-bus-lib/examples) directory for service files examples.
//!
//! ## Building
//! Build manually or use `cargo install krossbar-bus-hub` to install.
//!
//! Usage:
//! ```sh
//! Krossbar bus hub
//!
//! Usage: krossbar-bus-hub [OPTIONS]
//! Options:
//!   -l, --log-level <LOG_LEVEL>
//!           Log level: OFF, ERROR, WARN, INFO, DEBUG, TRACE [default: TRACE]
//!   -a, --additional-service-dirs <ADDITIONAL_SERVICE_DIRS>
//!           Additional service files directories [default: []]
//!   -s, --socket-path <SOCKET_PATH>
//!           Hub socket path [default: /var/run/krossbar.bus.socket]
//!   -h, --help
//!           Print help
//!   -V, --version
//!           Print version
//! ```

mod args;
mod hub;
mod permissions;
mod service_names;

use std::path::Path;

use clap::Parser;
use log::*;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    debug!("Starting Krossbar hub");

    let args = args::Args::parse();

    pretty_env_logger::formatted_builder()
        .filter_level(args.log_level)
        .init();

    for dir in args.additional_service_dirs.iter() {
        if !Path::new(&dir).exists() {
            return;
        }
    }

    hub::Hub::new(args).run().await;
}
