pub mod call_registry;
pub mod errors;
pub mod inspect_data;
pub mod messages;
pub mod monitor;
pub mod net;
pub mod service_names;

use std::env;

pub const SERVICE_FILES_DIR: &str = "/etc/karo/services";
pub const DEFAULT_HUB_SOCKET_PATH: &str = "/var/run/karo.bus.socket";
pub const HUB_SOCKET_PATH_ENV: &str = "CARO_HUB_SOCKET_PATH";

pub fn get_hub_socket_path() -> String {
    if let Ok(path) = env::var(HUB_SOCKET_PATH_ENV) {
        path
    } else {
        DEFAULT_HUB_SOCKET_PATH.into()
    }
}
