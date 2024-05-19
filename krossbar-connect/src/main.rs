use std::{path::PathBuf, str::FromStr};

use clap::{self, Parser};
use colored::*;
use futures::StreamExt;
use log::{LevelFilter, *};
use rustyline::{error::ReadlineError, ColorMode, Config, DefaultEditor, Result};
use serde_json::Value;

use krossbar_bus_common::{
    protocols::connect::{InspectData, INSPECT_METHOD},
    CONNECT_SERVICE_NAME, DEFAULT_HUB_SOCKET_PATH,
};
use krossbar_bus_lib::{Client, Service};

/// Krossbar bus connect
#[derive(Parser, Debug)]
#[clap(version, about, long_about = None)]
pub struct Args {
    /// Log level: OFF, ERROR, WARN, INFO, DEBUG, TRACE
    #[clap(short, long, default_value_t = LevelFilter::Warn)]
    pub log_level: log::LevelFilter,

    /// Service to connect to
    #[clap()]
    pub target_service: String,
}

fn print_help() {
    println!(
        "\t{} Inspect registered methods, signals, and states",
        "inspect".bright_blue()
    );
    println!(
        "\t{} {{method_name}} {{argument}} Call method with the given argument. Argument should be a JSON,",
        "call".bright_yellow()
    );
    println!(
        "\t\twhich deserializes into the method argument type (use tracing logs and `bus-monitor`"
    );
    println!("\t\tto find how method parameters serialize)");

    println!(
        "\t{} {{signal_name}} Subscribe on the signal",
        "subscribe".bright_yellow()
    );
    println!("\t{} Quit", "q".bright_blue());
}

fn format_response(endpoint_type: &str, endpoint_name: &str, json: &Value) {
    println!(
        "{} {} '{}' response: {}",
        ">".bright_blue(),
        endpoint_type,
        endpoint_name,
        json
    );
}

async fn handle_input_line(client: &mut Client, line: &String, service_name: &str) -> bool {
    let words: Vec<&str> = line.split(' ').collect();

    if words.len() == 0 {
        eprintln!("Empty input. Type 'help' to get list of commands");
    }

    if line == "help" {
        print_help()
    } else if line == "q" {
        // Exit
        return true;
    } else if words[0] == "call" {
        if words.len() != 3 {
            eprintln!("Ivalid number of 'call' arguments given. Use 'help' to see command syntax");
            return false;
        }

        let json = match Value::from_str(words[2]) {
            Ok(value) => value,
            Err(err) => {
                eprintln!(
                    "Failed to parse 'call' argument: {}. Should be a valid JSON",
                    err.to_string()
                );
                return false;
            }
        };

        match client.call(words[1], &json).await {
            Ok(response) => format_response("Method", words[1], &response),
            Err(err) => {
                eprintln!("Failed to make a call: {}", err.to_string())
            }
        }
    } else if words[0] == "subscribe" {
        if words.len() != 2 {
            eprintln!("Ivalid number of 'subscribe' arguments given: no signal name given. Use 'help' to see command syntax");
            return false;
        }

        let signal_name: String = words[1].into();
        match client.subscribe::<Value>(words[1]).await {
            Ok(mut stream) => {
                while let Some(value_result) = stream.next().await {
                    match value_result {
                        Ok(json) => format_response("Signal", &signal_name, &json),
                        Err(err) => {
                            eprintln!(
                                "Error subscribing to a signal '{}': {}",
                                words[1],
                                err.to_string()
                            );

                            return false;
                        }
                    }
                }
            }
            Err(err) => {
                eprintln!(
                    "Failed to subscribe to the signal '{}': {}",
                    words[1],
                    err.to_string()
                )
            }
        }
    } else if words[0] == "inspect" {
        match client.get::<InspectData>(INSPECT_METHOD).await {
            Ok(resp) => println!("{}", resp),
            Err(err) => eprintln!(
                "Failed to inspect service '{}': {}",
                service_name,
                err.to_string()
            ),
        }
    } else {
        eprintln!(
            "Unknown command '{}'. Type 'help' to get list of commands",
            words[0]
        )
    }

    return false;
}

#[tokio::main]
async fn main() -> Result<()> {
    debug!("Starting Krossbar bus connect");

    let args = Args::parse();

    pretty_env_logger::formatted_builder()
        .filter_level(args.log_level)
        .init();

    let mut bus = Service::new(
        CONNECT_SERVICE_NAME,
        &PathBuf::from(DEFAULT_HUB_SOCKET_PATH),
    )
    .await
    .expect("Failed to register connect service");

    debug!("Succesfully registered");

    let mut target_service = bus
        .connect_await(&args.target_service)
        .await
        .expect("Failed to connect to the target service");

    debug!("Succesfully connected to the service");
    print_help();

    let config = Config::builder().color_mode(ColorMode::Enabled).build();
    let mut rl = DefaultEditor::with_config(config)?;

    loop {
        let readline = rl.readline(&format!("{}", ">> ".bright_green()));
        match readline {
            Ok(line) => {
                if handle_input_line(&mut target_service, &line, &args.target_service).await {
                    return Ok(());
                }

                rl.add_history_entry(line.as_str()).unwrap();
            }
            Err(ReadlineError::Interrupted) => {
                println!("CTRL-C");
                break;
            }
            Err(ReadlineError::Eof) => {
                println!("CTRL-D");
                break;
            }
            Err(err) => {
                println!("Error: {:?}", err);
                break;
            }
        }
    }

    Ok(())
}
