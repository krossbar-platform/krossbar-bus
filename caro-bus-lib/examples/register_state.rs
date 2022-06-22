use std::time::Duration;

use log::LevelFilter;
use tokio;

use caro_bus_lib::BusConnection;

#[tokio::main]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Trace)
        .init();

    let mut bus = BusConnection::register("com.examples.register_state".into())
        .await
        .unwrap();

    let mut state = bus.register_state(&"state".into(), 42).unwrap();

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => { return },
            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                state.set(11);
                tokio::time::sleep(Duration::from_secs(1)).await;
                state.set(42);
                tokio::time::sleep(Duration::from_secs(1)).await;
                state.set(69);
            }
        }
    }
}
