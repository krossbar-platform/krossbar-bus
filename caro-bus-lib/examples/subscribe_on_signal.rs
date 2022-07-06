use log::{LevelFilter, *};
use tokio;

use caro_bus_lib::Bus;

#[tokio::main]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Debug)
        .init();

    let mut bus = Bus::register("com.examples.subscribe_on_signal")
        .await
        .unwrap();

    let mut peer_connection = bus
        .connect_await("com.examples.register_signal")
        .await
        .unwrap();

    peer_connection
        .subscribe("signal", |value: &i64| {
            debug!("Signal value: {}", value);
        })
        .await
        .unwrap();

    let _ = tokio::signal::ctrl_c().await;
}
