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
        "com.examples.register_signal",
        &PathBuf::from(DEFAULT_HUB_SOCKET_PATH),
    )
    .await
    .unwrap();

    let signal = service.register_signal::<i64>("signal").unwrap();

    tokio::spawn(service.run());

    let mut increment = 0;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => { return },
            _ = tokio::time::sleep(Duration::from_secs(3)) => {
                signal.emit(42 + increment).await.unwrap();
                signal.emit(11 + increment).await.unwrap();
                signal.emit(64 + increment).await.unwrap();
                increment += 1
            }
        }
    }
}
