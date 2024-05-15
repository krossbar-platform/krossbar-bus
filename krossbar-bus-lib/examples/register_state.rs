use std::{path::PathBuf, time::Duration};

use log::LevelFilter;
use tokio;

use krossbar_bus_common::DEFAULT_HUB_SOCKET_PATH;
use krossbar_bus_lib::service::Service;

#[tokio::main]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Trace)
        .init();

    let mut service = Service::new(
        "com.examples.register_state",
        &PathBuf::from(DEFAULT_HUB_SOCKET_PATH),
    )
    .await
    .unwrap();

    let mut state = service.register_state("state", 42).unwrap();

    let states = vec![11, 42, 69];
    let mut iter = states.into_iter().cycle();
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => { return },
            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                state.set(iter.next().unwrap()).await.unwrap();
            }
        }
    }
}
