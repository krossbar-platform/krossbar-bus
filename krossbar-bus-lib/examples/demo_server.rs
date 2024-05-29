use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use log::{info, LevelFilter};
use tokio::{self};

use krossbar_bus_common::DEFAULT_HUB_SOCKET_PATH;
use krossbar_bus_lib::service::Service;

#[tokio::main]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Info)
        .init();

    // Making a new Krossbar service
    let mut service = Service::new(
        "com.examples.demo_server",
        &PathBuf::from(DEFAULT_HUB_SOCKET_PATH),
    )
    .await
    .unwrap();

    // Register a state which holds each 10th client
    let mut state = service
        .register_state("10th_client", (0u64, String::new()))
        .unwrap();

    // Register a signal which emits when we've a new method call
    let signal = service.register_signal("client_count").unwrap();

    let client_counter = AtomicU64::from(0);

    // Register a method to greet clients
    service
        .register_async_method("greet", move |service_name, name: String| {
            info!("A call from \"{service_name}\": \"{name}\"");

            let client_count = client_counter.fetch_add(1, Ordering::Relaxed);

            // Note: `signal` and `state` methods to broadcast values don't capture `self`, which
            // allows moving the result futures into async block below
            let signal_sender = signal.clone().emit(client_count);
            let state_sender = if client_count % 10 == 0 {
                Some(state.set((client_count, name.clone())))
            } else {
                None
            };

            async move {
                signal_sender.await.unwrap();
                if let Some(state_sender) = state_sender {
                    state_sender.await.unwrap();
                }

                format!("Hello, {name}")
            }
        })
        .unwrap();

    // Register a method to receive one-way messages
    service
        .register_method("messages", move |service_name, name: String| {
            info!("A message from \"{service_name}\": \"{name}\"");
        })
        .unwrap();

    // Spawn a task to poll incoming data
    tokio::spawn(service.run());
    let _ = tokio::signal::ctrl_c().await;
}
