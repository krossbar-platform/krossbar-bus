mod client;
mod hub;
mod permissions;

use std::{
    io::{Error, ErrorKind},
    path::Path,
};

use clap::Parser;
use log::{LevelFilter, *};
use tokio::sync::mpsc;

use caro_bus_common::SERVICE_FILES_DIR;

/// Caro bus hub
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    debug!("Starting Caro hub");

    let args = Args::parse();

    pretty_env_logger::formatted_builder()
        .filter_level(args.log_level)
        .init();

    if !Path::new(&args.service_files_dir).exists() {
        return Err(Box::new(Error::new(
            ErrorKind::NotFound,
            format!(
                "Service files directory `{}` doesn't exist",
                &args.service_files_dir
            ),
        )) as Box<dyn std::error::Error>);
    }

    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>(1);

    let mut hub = hub::Hub::new(args, shutdown_rx);

    let result = tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            let _ = shutdown_tx.send(()).await;
            Ok(())
        }
        result = hub.run() => { result }
    };

    debug!("Shutting down Caro hub");

    result
}
