use std::{path::PathBuf, time::Duration};

use futures::StreamExt;
use log::{LevelFilter, *};
use tokio::{self, select};

use krossbar_bus_common::DEFAULT_HUB_SOCKET_PATH;
use krossbar_bus_lib::service::Service;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Info)
        .init();

    // Register new Krossbar service
    let mut service = Service::new(
        "com.examples.demo_client",
        &PathBuf::from(DEFAULT_HUB_SOCKET_PATH),
    )
    .await
    .unwrap();

    // Connect to a demo server
    let server_connection = service
        .connect_await("com.examples.demo_server")
        .await
        .unwrap();

    // Spawn a task to poll incoming messages
    // Note: you can move service handle into the task and continue using `Client` handle
    tokio::spawn(service.run());

    // Subscribe to call counter
    let mut subscription = server_connection
        .subscribe::<u64>("client_count")
        .await
        .unwrap();

    // Subscribe for each 10th client info
    let mut state_subscription = server_connection
        .subscribe::<(u64, String)>("10th_client")
        .await
        .unwrap();

    let mut counter = 0u64;
    'outer: loop {
        let client_name = format!("Client-{counter}");

        // Make a regular call
        let response: String = server_connection.call("greet", &client_name).await.unwrap();
        info!("Call response \"{response}\"");

        // Random frequence here
        if counter % 3 == 0 {
            // Get last 10th client
            let response: (u64, String) = server_connection.get("10th_client").await.unwrap();
            info!("10th client call \"{response:?}\"");
        }

        // Random frequence here too
        if counter % 7 == 0 {
            let message = format!("Message from {client_name}");

            // One-way message
            server_connection
                .message("messages", &message)
                .await
                .unwrap();
        }

        loop {
            select! {
                value = subscription.next() => {
                    info!("Client count: {value:?}");
                },
                value = state_subscription.next() => {
                    info!("10th client: {value:?}");
                },
                _ = tokio::time::sleep(Duration::from_secs(1)) => {
                    break;
                }
                _ = tokio::signal::ctrl_c() => {
                    break 'outer;
                }
            };
        }

        counter += 1;
    }
}
