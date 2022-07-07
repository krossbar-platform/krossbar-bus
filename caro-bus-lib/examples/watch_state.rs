#![feature(async_closure)]

use log::{LevelFilter, *};
use tokio;

use caro_bus_lib::Bus;

#[tokio::main]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Debug)
        .init();

    let mut bus = Bus::register("com.examples.watch_state").await.unwrap();

    let mut peer_connection = bus.connect("com.examples.register_state").await.unwrap();

    let current_state = peer_connection
        .watch("state", async move |value: i32| {
            debug!("New state value: {}", value);
        })
        .await
        .unwrap();

    debug!("Initial state {}", current_state);

    let _ = tokio::signal::ctrl_c().await;
}
