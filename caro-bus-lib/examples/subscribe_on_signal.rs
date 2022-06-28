use log::{LevelFilter, *};
use tokio;

use caro_bus_lib::BusConnection;

#[tokio::main]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Trace)
        .init();

    let mut bus = BusConnection::register("com.examples.subscribe_on_signal".into())
        .await
        .unwrap();

    let mut peer_connection = bus
        .connect("com.examples.register_signal".into())
        .await
        .unwrap();

    peer_connection
        .subscribe(&"signal".into(), |value: &i64| {
            debug!("Signal value: {}", value);
        })
        .await
        .unwrap();

    let _ = tokio::signal::ctrl_c().await;
}
