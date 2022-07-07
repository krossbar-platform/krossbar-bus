#![feature(async_closure)]

use log::LevelFilter;
use tokio;

use caro_bus_lib::Bus;

#[tokio::main]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Trace)
        .init();

    let mut bus = Bus::register("com.examples.register_method").await.unwrap();

    bus.register_method("method", async move |val: i32| -> String {
        format!("Hello, {}", val).into()
    })
    .unwrap();

    let _ = tokio::signal::ctrl_c().await;
}
