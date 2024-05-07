use log::LevelFilter;
use tokio;

use krossbar_bus_lib::Bus;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Warn)
        .init();

    let mut bus = Bus::register("com.examples.call_method").await.unwrap();

    let peer_connection = bus
        .connect("com.examples.register_method".into())
        .await
        .unwrap();

    const NUM_CALLS: usize = 25_000;

    let start = std::time::Instant::now();

    for _ in 0..NUM_CALLS {
        let _: i32 = peer_connection.call("method", &42).await.unwrap();
    }

    let duration = std::time::Instant::now() - start;
    println!(
        "{} calls made in {} milliseconds",
        NUM_CALLS,
        duration.as_millis()
    );
}
