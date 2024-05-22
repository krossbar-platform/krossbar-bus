use std::{path::PathBuf, time::Duration};

use log::{LevelFilter, *};
use tokio;

use krossbar_bus_common::DEFAULT_HUB_SOCKET_PATH;
use krossbar_bus_lib::service::Service;

#[tokio::main]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Debug)
        .init();

    let mut service = Service::new(
        "com.examples.call_state",
        &PathBuf::from(DEFAULT_HUB_SOCKET_PATH),
    )
    .await
    .unwrap();

    let peer_connection = service
        .connect("com.examples.register_state".into())
        .await
        .unwrap();

    tokio::spawn(service.run());

    let call_result = peer_connection.get::<i32>("state").await.unwrap();
    debug!("State call result: {}", call_result);

    tokio::time::sleep(Duration::from_secs(1)).await;

    let call_result = peer_connection.get::<i32>("state").await.unwrap();
    debug!("State call result: {}", call_result);

    tokio::time::sleep(Duration::from_secs(1)).await;

    let call_result = peer_connection.get::<i32>("state").await.unwrap();
    debug!("State call result: {}", call_result);

    tokio::time::sleep(Duration::from_secs(1)).await;
}
