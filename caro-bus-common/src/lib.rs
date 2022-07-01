pub mod call_registry;
pub mod errors;
pub mod messages;
pub mod monitor;
pub mod net;
pub mod service_names;

pub const SERVICE_FILES_DIR: &str = "/etc/caro.services.d";
pub const HUB_SOCKET_PATH: &str = "/var/run/caro/bus.socket";
