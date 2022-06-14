mod client;
mod hub;
mod permissions;

use log::{LevelFilter, *};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    debug!("Starting Caro hub");

    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Trace)
        .init();

    let (shutdown_tx, shutwon_rx) = mpsc::channel::<()>(1);

    let mut hub = hub::Hub::new(shutwon_rx);

    let result = tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            let _ = shutdown_tx.send(()).await;
            Ok(())
        }
        result = hub.run() => { result }
    };

    debug!("Shutting down Caro hub");

    result
}
