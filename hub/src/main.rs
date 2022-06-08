mod hub;
mod client;
mod permissions;

use log::LevelFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Info)
        .init();

    let mut hub = hub::Hub::new();
    hub.run().await
}
