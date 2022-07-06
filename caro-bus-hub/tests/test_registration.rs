use std::{path::Path, time::Duration};

use json::JsonValue;
use log::LevelFilter;
use tempdir::TempDir;
use tokio::{
    fs::OpenOptions,
    io::AsyncWriteExt,
    net::UnixStream,
    sync::mpsc::{self, Sender},
    time,
};

use caro_bus_common::{HUB_SOCKET_PATH, SERVICE_FILES_DIR};
use caro_bus_hub::{args::Args, hub::Hub};
use caro_bus_lib::Bus;

async fn start_hub(service_files_dir: &str) -> Sender<()> {
    let args = Args {
        log_level: LevelFilter::Debug,
        service_files_dir: service_files_dir.into(),
    };

    // Uncomment for logging. Doesn't work with multiple tests
    // pretty_env_logger::formatted_builder()
    //     .filter_level(args.log_level)
    //     .init();

    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>(1);

    tokio::spawn(async move {
        let mut hub = Hub::new(args, shutdown_rx);
        hub.run().await.expect("Failed to run hub");

        println!("Shutting hub down");
    });

    println!("Succesfully started hub socket");
    shutdown_tx
}

async fn write_service_file(service_dir: &Path, service_name: &str, content: JsonValue) {
    let service_file_path = service_dir.join(format!("{}.service", service_name));

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .open(service_file_path.as_path())
        .await
        .expect("Failed to create service file");

    file.write_all(json::stringify(content).as_bytes())
        .await
        .expect("Failed to write service file content");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_dropped_connection() {
    let shutdown_tx = start_hub(SERVICE_FILES_DIR).await;
    // Lets wait until hub starts
    time::sleep(Duration::from_millis(10)).await;

    let mut connection = UnixStream::connect(HUB_SOCKET_PATH)
        .await
        .expect("Failed to connect to the hub");
    println!("Succesfulyy connected to the hub");

    // Force drop connection
    drop(connection);

    // Try to connect one more time to check if hub is still working
    connection = UnixStream::connect(HUB_SOCKET_PATH)
        .await
        .expect("Failed to connect to the hub");
    println!("Succesfulyy connected to the hub second time");
    drop(connection);

    shutdown_tx
        .send(())
        .await
        .expect("Failed to send shutdown request to the hub");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_non_existing_service_name() {
    let service_dir =
        TempDir::new("caro_test_non_existing_service_name").expect("Failed to create tempdir");

    let shutdown_tx = start_hub(service_dir.path().as_os_str().to_str().unwrap()).await;
    // Lets wait until hub starts
    time::sleep(Duration::from_millis(10)).await;

    match Bus::register("non.existing.name").await {
        Ok(_) => panic!("Shouldn't be allowed"),
        Err(err) => {
            println!("Valid connection error: {}", err.to_string())
        }
    }

    shutdown_tx
        .send(())
        .await
        .expect("Failed to send shutdown request to the hub");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_non_allowed_service_name() {
    let service_dir =
        TempDir::new("test_non_allowed_service_name").expect("Failed to create tempdir");

    // Create service file first
    let service_file_json = json::parse(
        r#"
    {
        "exec": "/usr/bin/*",
        "incoming_connections": ["**"]
    }
    "#,
    )
    .unwrap();

    let service_name = "not.allowed.name";
    write_service_file(service_dir.path(), service_name, service_file_json).await;

    let shutdown_tx = start_hub(service_dir.path().as_os_str().to_str().unwrap()).await;
    // Lets wait until hub starts
    time::sleep(Duration::from_millis(10)).await;

    match Bus::register(service_name).await {
        Ok(_) => panic!("Shouldn't be allowed"),
        Err(err) => {
            println!("Valid connection error: {}", err.to_string())
        }
    }

    shutdown_tx
        .send(())
        .await
        .expect("Failed to send shutdown request to the hub");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_valid_registration() {
    let service_dir = TempDir::new("test_valid_registration").expect("Failed to create tempdir");

    // Create service file first
    let service_file_json = json::parse(
        r#"
    {
        "exec": "/**/*",
        "incoming_connections": ["**"]
    }
    "#,
    )
    .unwrap();

    let service_name = "com.caro.service.name";
    write_service_file(service_dir.path(), service_name, service_file_json).await;

    let shutdown_tx = start_hub(service_dir.path().as_os_str().to_str().unwrap()).await;
    // Lets wait until hub starts
    time::sleep(Duration::from_millis(10)).await;

    Bus::register(service_name)
        .await
        .expect("Failed to register valid service");

    shutdown_tx
        .send(())
        .await
        .expect("Failed to send shutdown request to the hub");
}
