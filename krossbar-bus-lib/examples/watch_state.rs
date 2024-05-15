use std::path::PathBuf;

use futures::StreamExt;
use krossbar_bus_common::DEFAULT_HUB_SOCKET_PATH;
use krossbar_bus_lib::service::Service;
use log::{LevelFilter, *};
use tokio::{self, select};

#[tokio::main]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Trace)
        .init();

    let mut service = Service::new(
        "com.examples.watch_state",
        &PathBuf::from(DEFAULT_HUB_SOCKET_PATH),
    )
    .await
    .unwrap();

    let peer_connection = service
        .connect("com.examples.register_state")
        .await
        .unwrap();

    let mut subscription = peer_connection.subscribe::<i32>("state").await.unwrap();

    loop {
        select! {
            value = subscription.next() => {
                debug!("State value: {value:?}");
            }
            _ = tokio::signal::ctrl_c() => {
                break;
            }
        }
    }
}
