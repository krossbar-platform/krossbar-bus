//! Krossbar bus common data lib
//!
//! See [krossbar-bus-lib](https://crates.io/crates/krossbar-bus-lib) for more details
pub mod protocols;

pub const MONITOR_SERVICE_NAME: &str = "krossbar.monitor";
pub const CONNECT_SERVICE_NAME: &str = "krossbar.connect";

pub const DEFAULT_HUB_SOCKET_PATH: &str = "/var/run/krossbar.bus.socket";
pub const DEFAULT_SERVICE_FILES_DIR: &str = "/etc/krossbar/services/";
