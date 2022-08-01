use std::time::Duration;

use log::LevelFilter;
use tokio;

use karo_bus_lib::Bus;

#[tokio::main]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Trace)
        .init();

    let mut bus = Bus::register("com.examples.register_state").await.unwrap();

    let mut state = bus.register_state("state", 42).unwrap();

    let states = vec![11, 42, 69];
    let mut iter = states.into_iter().cycle();
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => { return },
            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                state.set(iter.next().unwrap());
            }
        }
    }
}
