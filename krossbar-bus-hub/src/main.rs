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
