use std::path::PathBuf;

use clap::{self, Parser};
use colored::*;
use krossbar_bus_common::{DEFAULT_HUB_SOCKET_PATH, MONITOR_SERVICE_NAME};
use krossbar_bus_lib::Service;
use krossbar_common_rpc::monitor::MonitorMessage;
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

fn short_name(name: &String, target_service: bool) -> String {
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

async fn handle_message(monitor_message: &MonitorMessage) {
    let message = bson::from_bson::<Message>(monitor_message.message.clone())
        .expect("Failed to deserialize bus message ");

    let direction_symbol = match monitor_message.direction {
        MonitorMessageDirection::Incoming => "<<<".bright_green(),
        MonitorMessageDirection::Outgoing => ">>>".bright_blue(),
    };

    println!(
        "{} {} {}: {}{}{} {}",
        short_name(
            &monitor_message.receiver,
            monitor_message.direction == MonitorMessageDirection::Incoming
        ),
        direction_symbol,
        short_name(
            &monitor_message.sender,
            monitor_message.direction == MonitorMessageDirection::Outgoing
        ),
        "[".bright_yellow(),
        message.seq(),
        "]".bright_yellow(),
        message.body()
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
        .register_method(MONITOR_METHOD, |message: MonitorMessage| async move {
            handle_message(&message)
        })
        .expect("Failed to register signalling function");

    let _peer = service
        .connect_await(&args.target_service)
        .await
        .expect("Failed to connect to the target service");

    debug!("Succesfully connected to the peer. Listening");
    let _ = tokio::signal::ctrl_c().await;
    debug!("Shutting down");

    Ok(())
}
