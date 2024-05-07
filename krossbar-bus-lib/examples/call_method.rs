use log::{LevelFilter, *};
use tokio;

use krossbar_bus_lib::Bus;

#[tokio::main]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Debug)
        .init();

    let mut bus = Bus::register("com.examples.call_method").await.unwrap();

    let peer_connection = bus
        .connect("com.examples.register_method".into())
        .await
        .unwrap();

    let call_result: String = peer_connection.call("method", &42).await.unwrap();
    debug!("Method call result: {}", call_result);

    let call_result: String = peer_connection.call("method", &11).await.unwrap();
    debug!("Method call result: {}", call_result);

    let call_result: String = peer_connection.call("method", &69).await.unwrap();
    debug!("Method call result: {}", call_result);
}
