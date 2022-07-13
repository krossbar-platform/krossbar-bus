use clap::{self, Parser};
use colored::*;
use log::{LevelFilter, *};

use caro_bus_common::{
    messages::Message,
    monitor::{MonitorMessage, MonitorMessageDirection, MONITOR_METHOD, MONITOR_SERVICE_NAME},
};
use caro_bus_lib::Bus;

/// Caro bus monitor
#[derive(Parser, Debug)]
#[clap(version, about, long_about = None)]
pub struct Args {
    /// Log level: OFF, ERROR, WARN, INFO, DEBUG, TRACE
    #[clap(short, long, value_parser, default_value_t = LevelFilter::Warn)]
    pub log_level: log::LevelFilter,

    /// Service to monitor
    #[clap(value_parser)]
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

fn handle_message(monitor_message: &MonitorMessage) {
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    debug!("Starting Caro hub");

    let args = Args::parse();

    pretty_env_logger::formatted_builder()
        .filter_level(args.log_level)
        .init();

    let mut bus = Bus::register(MONITOR_SERVICE_NAME)
        .await
        .expect("Failed to register monitor");

    bus.register_method(MONITOR_METHOD, |message: MonitorMessage| {
        Box::pin(async move { handle_message(&message) })
    })
    .expect("Failed to register signalling function");

    let _peer = bus
        .connect_await(&args.target_service)
        .await
        .expect("Failed to connect to the target service");

    debug!("Succesfully connected to the peer. Listening");
    let _ = tokio::signal::ctrl_c().await;
    debug!("Shutting down");

    Ok(())
}
