use std::path::PathBuf;

use log::LevelFilter;
use tokio;

use krossbar_bus_common::DEFAULT_HUB_SOCKET_PATH;
use krossbar_bus_lib::service::Service;

#[tokio::main]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Warn)
        .init();

    let mut service = Service::new(
        "com.examples.register_method",
        &PathBuf::from(DEFAULT_HUB_SOCKET_PATH),
    )
    .await
    .unwrap();

    service
        .register_method("method", |client_name: String, val: i32| {
            println!("Got a call from {client_name}");

            val
        })
        .unwrap();

    let _ = tokio::signal::ctrl_c().await;
}
