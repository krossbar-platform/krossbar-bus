use std::path::PathBuf;

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
        "com.examples.call_method",
        &PathBuf::from(DEFAULT_HUB_SOCKET_PATH),
    )
    .await
    .unwrap();

    let peer_connection = service
        .connect("com.examples.register_method".into())
        .await
        .unwrap();

    tokio::spawn(service.run());

    let call_result: String = peer_connection.call("method", &42).await.unwrap();
    debug!("Method call result: {}", call_result);

    let call_result: String = peer_connection.call("method", &11).await.unwrap();
    debug!("Method call result: {}", call_result);

    let call_result: String = peer_connection.call("method", &69).await.unwrap();
    debug!("Method call result: {}", call_result);
}
