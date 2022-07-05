use std::{
    io::{self, Write},
    str::FromStr,
};

use clap::{self, Parser};
use colored::*;
use log::{LevelFilter, *};
use serde_json::Value;

use caro_bus_common::connect::CONNECT_SERVICE_NAME;
use caro_bus_lib::{peer::Peer, Bus};

/// Caro bus connect
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
    println!("\t{} Quit", "q".bright_yellow());
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

async fn handle_input_line(service: &mut Peer, line: String) -> bool {
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

        match service.call(&words[1].into(), &json).await {
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
            .subscribe(&words[1].into(), move |json: &Value| {
                format_response("Signal", &signal_name, json);
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
            .watch(&words[1].into(), move |json: &Value| {
                format_response("State", &state_name, json);
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
    } else {
        println!("Unknown command. Type 'help' to get list of commands")
    }

    return false;
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    debug!("Starting Caro bus connect");

    let args = Args::parse();

    pretty_env_logger::formatted_builder()
        .filter_level(args.log_level)
        .init();

    let mut bus = Bus::register(&CONNECT_SERVICE_NAME.into())
        .await
        .expect("Failed to register connect service");

    debug!("Succesfully registered");

    let mut peer = bus
        .connect_await(args.target_service)
        .await
        .expect("Failed to connect to the target service");

    debug!("Succesfully connected to the service");

    let input_promt = || print!("{}", "> ".bright_green());

    input_promt();
    io::stdout().flush().unwrap();
    for line_result in io::stdin().lines() {
        if handle_input_line(&mut peer, line_result?).await {
            return Ok(());
        }

        input_promt();
        io::stdout().flush().unwrap();
    }

    Ok(())
}
