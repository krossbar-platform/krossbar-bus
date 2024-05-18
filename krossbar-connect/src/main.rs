use std::str::FromStr;

use clap::{self, Parser};
use colored::*;
use log::{LevelFilter, *};
use rustyline::error::ReadlineError;
use rustyline::{ColorMode, Config, Editor, Result};
use serde_json::Value;

use krossbar_bus_common::inspect_data::CONNECT_SERVICE_NAME;
use krossbar_bus_lib::{peer::Peer, Bus};

/// Krossbar bus connect
#[derive(Parser, Debug)]
#[clap(version, about, long_about = None)]
pub struct Args {
    /// Log level: OFF, ERROR, WARN, INFO, DEBUG, TRACE
    #[clap(short, long, value_parser, default_value_t = LevelFilter::Warn)]
    pub log_level: log::LevelFilter,

    /// Service to connect to
    #[clap(value_parser)]
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
    println!(
        "\t{} {{state_name}} Watch for the state changes",
        "watch".bright_yellow()
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

async fn handle_input_line(service: &mut Peer, line: &String) -> bool {
    let words: Vec<&str> = line.split(' ').collect();

    if words.len() == 0 {
        println!("Empty input. Type 'help' to get list of commands");
    }

    if line == "help" {
        print_help()
    } else if line == "q" {
        // Exit
        return true;
    } else if words[0] == "call" {
        if words.len() != 3 {
            println!("Ivalid number of 'call' arguments given. Use 'help' to see command syntax");
            return false;
        }

        let json = match Value::from_str(words[2]) {
            Ok(value) => value,
            Err(err) => {
                println!(
                    "Failed to parse 'call' argument: {}. Should be a valid JSON",
                    err.to_string()
                );
                return false;
            }
        };

        match service.call(words[1], &json).await {
            Ok(response) => format_response("Method", words[1], &response),
            Err(err) => {
                println!("Failed to make a call: {}", err.to_string())
            }
        }
    } else if words[0] == "subscribe" {
        if words.len() != 2 {
            println!("Ivalid number of 'subscribe' arguments given: no signal name given. Use 'help' to see command syntax");
            return false;
        }

        let signal_name: String = words[1].into();
        if let Err(err) = service
            .subscribe(words[1], move |json: Value| {
                let endpoint_name = signal_name.clone();
                async move {
                    format_response("Signal", &endpoint_name, &json);
                }
            })
            .await
        {
            println!(
                "Failed to subscribe to the signal '{}': {}",
                words[1],
                err.to_string()
            )
        }
    } else if words[0] == "watch" {
        if words.len() != 2 {
            println!("Ivalid number of 'watch' arguments given: no state name given. Use 'help' to see command syntax");
            return false;
        }

        let state_name: String = words[1].into();
        match service
            .watch(words[1], move |json: Value| {
                let endpoint_name = state_name.clone();
                async move {
                    format_response("State", &endpoint_name, &json);
                }
            })
            .await
        {
            Ok(value) => format_response("State", words[1], &value),
            Err(err) => println!(
                "Failed to subscribe to the state '{}': {}",
                words[1],
                err.to_string()
            ),
        }
    } else if words[0] == "inspect" {
        match service
            .call::<(), krossbar_bus_common::inspect_data::InspectData>(
                krossbar_bus_common::inspect_data::INSPECT_METHOD,
                &(),
            )
            .await
        {
            Ok(resp) => println!("{}", resp),
            Err(err) => println!(
                "Failed to inspect service '{}': {}",
                service.name(),
                err.to_string()
            ),
        }
    } else {
        println!(
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

    let mut bus = Bus::register(CONNECT_SERVICE_NAME)
        .await
        .expect("Failed to register connect service");

    debug!("Succesfully registered");

    let mut peer = bus
        .connect_await(&args.target_service)
        .await
        .expect("Failed to connect to the target service");

    debug!("Succesfully connected to the service");
    print_help();

    let config = Config::builder().color_mode(ColorMode::Enabled).build();
    let mut rl = Editor::<()>::with_config(config)?;

    loop {
        let readline = rl.readline(&format!("{}", ">> ".bright_green()));
        match readline {
            Ok(line) => {
                if handle_input_line(&mut peer, &line).await {
                    return Ok(());
                }

                rl.add_history_entry(line.as_str());
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
