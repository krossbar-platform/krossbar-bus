//! ## Krossbar bus monitor
//!
//! Krossbar monitor allows connecting to Krossbar services to monitor message exchange.
//!
//! **Note**: To be able to use monitor, you need corresponding features, which are enabled bu default:
//! - `privileged-services` hub feature, which allows using Krossbar tools;
//! - `monitor` Krossbar bus library feature, which adds monitoring support to a service.
//!
//! ## Usage
//! ```sh
//! Krossbar bus monitor
//!
//! Usage: krossbar-monitor [OPTIONS] <TARGET_SERVICE>
//!
//! Arguments:
//!   <TARGET_SERVICE>  Service to monitor
//!
//! Options:
//!   -l, --log-level <LOG_LEVEL>  Log level: OFF, ERROR, WARN, INFO, DEBUG, TRACE [default: WARN]
//!   -h, --help                   Print help
//!   -V, --version                Print version
//! ```
//!

use std::path::PathBuf;

use clap::{self, Parser};
use colored::*;
use krossbar_bus_common::{DEFAULT_HUB_SOCKET_PATH, MONITOR_SERVICE_NAME};
use krossbar_bus_lib::Service;
use krossbar_common_rpc::monitor::{Direction, MonitorMessage, MESSAGE_METHOD};
use log::{LevelFilter, *};

/// Krossbar bus monitor
#[derive(Parser, Debug)]
#[clap(version, about, long_about = None)]
pub struct Args {
    /// Log level: OFF, ERROR, WARN, INFO, DEBUG, TRACE
    #[clap(short, long, default_value_t = LevelFilter::Warn)]
    pub log_level: log::LevelFilter,

    /// Service to monitor
    #[clap()]
    pub target_service: String,
}

fn short_name(name: &str, target_service: bool) -> String {
    let sections: Vec<&str> = name.split('.').collect();
    let mut result = String::new();

    for (i, section) in sections.iter().enumerate() {
        if i < (sections.len() - 1) {
            result.push(section.chars().nth(0).unwrap());
            result.push('.');
        } else {
            let last_section = if target_service {
                format!("{}", section.bright_red())
            } else {
                (*section).into()
            };

            result.push_str(&last_section);
        }
    }

    result
}

async fn handle_message(service_name: &str, monitor_message: &MonitorMessage) {
    let (direction_symbol, self_is_target) = match monitor_message.direction {
        Direction::Incoming => ("<<<".bright_green(), true),
        Direction::Outgoing => (">>>".bright_blue(), false),
    };

    println!(
        "{} {} {}: {}{}{} {:?}",
        short_name(service_name, self_is_target),
        direction_symbol,
        short_name(&monitor_message.peer_name, !self_is_target),
        "[".bright_yellow(),
        monitor_message.message.id,
        "]".bright_yellow(),
        monitor_message.message.data
    );
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    debug!("Starting Krossbar monitor");

    let args = Args::parse();

    pretty_env_logger::formatted_builder()
        .filter_level(args.log_level)
        .init();

    let mut service = Service::new(
        MONITOR_SERVICE_NAME,
        &PathBuf::from(DEFAULT_HUB_SOCKET_PATH),
    )
    .await
    .expect("Failed to register monitor");

    service
        .register_method(
            MESSAGE_METHOD,
            |service_name: String, message: MonitorMessage| async move {
                handle_message(&service_name, &message).await
            },
        )
        .expect("Failed to register signalling function");

    let _peer = service
        .connect_await(&args.target_service)
        .await
        .expect("Failed to connect to the target service");

    debug!("Succesfully connected to the peer. Listening");

    let _ = tokio::signal::ctrl_c().await;
    debug!("Shutting monitor down");

    Ok(())
}
