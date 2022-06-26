use log::{LevelFilter, *};
use tokio;

use caro_bus_lib::BusConnection;

#[tokio::main]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Debug)
        .init();

    let mut bus = BusConnection::register("com.examples.call_method".into())
        .await
        .unwrap();

    let mut peer_connection = bus
        .connect("com.examples.register_method".into())
        .await
        .unwrap();

    let call_result: String = peer_connection.call(&"method".into(), &42).await.unwrap();
    debug!("Method call result: {}", call_result);

    let call_result: String = peer_connection.call(&"method".into(), &11).await.unwrap();
    debug!("Method call result: {}", call_result);

    let call_result: String = peer_connection.call(&"method".into(), &69).await.unwrap();
    debug!("Method call result: {}", call_result);
}
