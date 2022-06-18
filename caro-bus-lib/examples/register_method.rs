use log::LevelFilter;
use tokio;

use caro_bus_lib::BusConnection;

#[tokio::main]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Trace)
        .init();

    let mut bus = BusConnection::register("com.examples.register_method".into())
        .await
        .unwrap();

    bus.register_method(&"method".into(), |val: &i32| -> String {
        format!("Hello, {}", val).into()
    })
    .unwrap();

    let _ = tokio::signal::ctrl_c().await;
    bus.close().await;
}
