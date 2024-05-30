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

use bson::Bson;
use clap::{self, Parser};
use colored::*;
use krossbar_bus_common::{DEFAULT_HUB_SOCKET_PATH, MONITOR_SERVICE_NAME};
use krossbar_bus_lib::Service;
use krossbar_common_rpc::{
    monitor::{Direction, MonitorMessage, MESSAGE_METHOD},
    RpcData,
};
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
            result.push(section.chars().next().unwrap());
            result.push('.');
        } else {
            let last_section = if target_service {
                format!("{}", section.red())
            } else {
                (*section).into()
            };

            result.push_str(&last_section);
        }
    }

    result
}

// pub enum RpcData {
//     /// One way message
//     Message { endpoint: String, body: Bson },
//     /// RPC call
//     Call { endpoint: String, params: Bson },
//     /// Subscription request
//     Subscription { endpoint: String },
//     /// Connection request
//     /// `client_name` - initiator client name
//     /// `target_name` - connection target name, which can be used
//     ///     to implement gateways
//     ConnectionRequest {
//         client_name: String,
//         target_name: String,
//     },
//     /// Message response
//     Response(crate::Result<Bson>),
//     /// Mesage response, which precedes incoming FD
//     FdResponse(crate::Result<Bson>),
// }
fn format_data(data: RpcData) -> String {
    let format_result = |value: krossbar_bus_lib::Result<Bson>| -> String {
        match value {
            Ok(body) => format!(
                "{} {}",
                "OK".green(),
                body.into_relaxed_extjson().to_string()
            ),
            Err(e) => format!("{} {}", "ERR".red(), e.to_string()),
        }
    };

    match data {
        RpcData::Message { endpoint, body } => {
            format!(
                "{}({}) -> {}",
                "Message".yellow(),
                body.into_relaxed_extjson().to_string(),
                endpoint.white(),
            )
        }
        RpcData::Call { endpoint, params } => {
            format!(
                "{}({}) -> {}",
                "Call".blue(),
                params.into_relaxed_extjson().to_string(),
                endpoint.white(),
            )
        }
        RpcData::Subscription { endpoint } => {
            format!("{} -> {}", "Subscription".purple(), endpoint.white(),)
        }
        RpcData::ConnectionRequest {
            client_name,
            target_name,
        } => {
            format!("{} {client_name} -> {target_name}", "Connection".magenta())
        }
        RpcData::Response(response) => {
            format!("{} <- {}", format_result(response), "Response".red(),)
        }

        RpcData::FdResponse(response) => {
            format!("{} <- {}", format_result(response), "FD response".red(),)
        }
    }
}

async fn handle_message(service_name: &str, monitor_message: &MonitorMessage) {
    let (direction_symbol, self_is_target) = match monitor_message.direction {
        Direction::Incoming => ("<<<".green(), true),
        Direction::Outgoing => (">>>".blue(), false),
    };

    println!(
        "{} {} {}: {}{}{} {}",
        short_name(service_name, self_is_target),
        direction_symbol,
        short_name(&monitor_message.peer_name, !self_is_target),
        "[".yellow(),
        monitor_message.message.id,
        "]".yellow(),
        format_data(monitor_message.message.data.clone())
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

    tokio::spawn(service.run());

    let _ = tokio::signal::ctrl_c().await;
    debug!("Shutting monitor down");

    Ok(())
}
