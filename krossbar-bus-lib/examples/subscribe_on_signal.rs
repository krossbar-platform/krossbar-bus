use std::path::PathBuf;

use futures::StreamExt;
use log::{LevelFilter, *};
use tokio::{self, select};

use krossbar_bus_common::DEFAULT_HUB_SOCKET_PATH;
use krossbar_bus_lib::service::Service;

#[tokio::main]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Debug)
        .init();

    let mut service = Service::new(
        "com.examples.subscribe_on_signal",
        &PathBuf::from(DEFAULT_HUB_SOCKET_PATH),
    )
    .await
    .unwrap();

    let peer_connection = service
        .connect_await("com.examples.register_signal")
        .await
        .unwrap();

    let mut subscription = peer_connection.subscribe::<i64>("signal").await.unwrap();

    loop {
        select! {
            value = subscription.next() => {
                debug!("Signal value: {value:?}");
            }
            _ = tokio::signal::ctrl_c() => {
                break;
            }
        }
    }
}
