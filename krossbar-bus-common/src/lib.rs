pub mod inspect_data;
pub mod message;

pub const HUB_CONNECT_METHOD: &str = "connect";
pub const HUB_REGISTER_METHOD: &str = "register";

pub const MONITOR_SERVICE_NAME: &str = "krossbar.monitor";
pub const CONNECT_SERVICE_NAME: &str = "krossbar.connect";

pub const DEFAULT_HUB_SOCKET_PATH: &str = "/var/run/krossbar.bus.socket";
pub const DEFAULT_SERVICE_FILES_DIR: &str = "/etc/krossbar/services";
