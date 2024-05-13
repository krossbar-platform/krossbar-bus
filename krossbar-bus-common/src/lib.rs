pub mod inspect_data;
pub mod service_names;
pub mod message;

use std::env;

pub const HUB_CONNECT_METHOD: &str = "connect";
pub const HUB_REGISTER_METHOD: &str = "register";

pub const MONITOR_SERVICE_NAME: &str = "krossbar.monitor";
pub const CONNECT_SERVICE_NAME: &str = "krossbar.connect";

pub const HUB_SOCKET_PATH_ENV: &str = "KROSSBAR_HUB_SOCKET_PATH";
pub const SERVICE_FILES_DIR_ENV: &str = "KROSSBAR_SERVICE_DIRS_PATH";

const DEFAULT_HUB_SOCKET_PATH: &str = "/var/run/krossbar.bus.socket";
const DEFAULT_SERVICE_FILES_DIR: &str = "/etc/krossbar/services";

pub fn get_hub_socket_path() -> String {
    if let Ok(path) = env::var(HUB_SOCKET_PATH_ENV) {
        path
    } else {
        DEFAULT_HUB_SOCKET_PATH.into()
    }
}

pub fn get_service_files_dir() -> String {
    if let Ok(path) = env::var(SERVICE_FILES_DIR_ENV) {
        path
    } else {
        DEFAULT_SERVICE_FILES_DIR.into()
    }
}
