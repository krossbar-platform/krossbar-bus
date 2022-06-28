use std::time::Duration;

use log::LevelFilter;
use tokio;

use caro_bus_lib::BusConnection;

#[tokio::main]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Trace)
        .init();

    let mut bus = BusConnection::register("com.examples.register_signal".into())
        .await
        .unwrap();

    let signal = bus.register_signal::<i64>(&"signal".into()).unwrap();

    let mut increment = 0;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => { return },
            _ = tokio::time::sleep(Duration::from_secs(3)) => {
                signal.emit(42 + increment);
                signal.emit(11 + increment);
                signal.emit(64 + increment);
                increment += 1
            }
        }
    }
}
