use std::path::PathBuf;

use clap::Parser;
use log::LevelFilter;

use krossbar_bus_common::DEFAULT_HUB_SOCKET_PATH;

#[derive(Parser, Debug)]
#[clap(version, about, long_about = None)]
/// Krossbar bus hub
pub struct Args {
    /// Log level: OFF, ERROR, WARN, INFO, DEBUG, TRACE
    #[clap(short, long, default_value_t = LevelFilter::Trace)]
    pub log_level: log::LevelFilter,

    /// Additional service files directories
    #[clap(short, long, default_value = "[]")]
    pub additional_service_dirs: Vec<PathBuf>,

    /// Hub socket path
    #[clap(short, long, default_value = DEFAULT_HUB_SOCKET_PATH)]
    pub socket_path: PathBuf,
}
