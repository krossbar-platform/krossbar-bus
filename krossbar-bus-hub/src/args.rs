use clap::Parser;
use log::LevelFilter;

use krossbar_bus_common::SERVICE_FILES_DIR;

/// Krossbar bus hub
#[derive(Parser, Debug)]
#[clap(version, about, long_about = None)]
pub struct Args {
    /// Log level: OFF, ERROR, WARN, INFO, DEBUG, TRACE
    #[clap(short, long, value_parser, default_value_t = LevelFilter::Trace)]
    pub log_level: log::LevelFilter,

    /// Number of times to greet
    #[clap(short, long, value_parser, default_value_t = SERVICE_FILES_DIR.into())]
    pub service_files_dir: String,
}
