use log::{LevelFilter, *};
use tokio;

use karo_bus_lib::Bus;

#[tokio::main]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Trace)
        .init();

    let mut bus = Bus::register("com.examples.watch_state").await.unwrap();

    let peer_connection = bus.connect("com.examples.register_state").await.unwrap();

    let current_state = peer_connection
        .watch("state", |value: i32| async move {
            debug!("New state value: {}", value);
        })
        .await
        .unwrap();

    debug!("Initial state {}", current_state);

    let _ = tokio::signal::ctrl_c().await;
}
