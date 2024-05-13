use clap::Parser;
use log::LevelFilter;

/// Krossbar bus hub
#[derive(Parser, Debug)]
#[clap(version, about, long_about = None)]
pub struct Args {
    /// Log level: OFF, ERROR, WARN, INFO, DEBUG, TRACE
    #[clap(short, long, value_parser, default_value_t = LevelFilter::Trace)]
    pub log_level: log::LevelFilter,

    /// Additional service files directories
    #[clap(short, long, value_parser)]
    pub additional_service_dirs: Vec<String>,
}
