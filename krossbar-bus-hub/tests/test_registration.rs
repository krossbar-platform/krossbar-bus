use std::{env, path::Path, time::Duration};

use json::JsonValue;
use krossbar_bus_common::{get_service_files_dir, HUB_SOCKET_PATH_ENV};
use krossbar_bus_lib::service::Service;
use krossbar_hub_lib::{args::Args, hub::Hub};
use log::LevelFilter;
use tempdir::TempDir;
use tokio::{fs::OpenOptions, io::AsyncWriteExt, net::UnixStream, time};
use tokio_util::sync::CancellationToken;

fn start_hub(socket_path: &str, service_files_dir: &str) -> CancellationToken {
    env::set_var(HUB_SOCKET_PATH_ENV, socket_path);

    let args = Args {
        log_level: LevelFilter::Debug,
        additional_service_dirs: vec![service_files_dir.to_owned()],
    };

    let _ = pretty_env_logger::formatted_builder()
        .filter_level(args.log_level)
        .try_init();

    let cancel_token = CancellationToken::new();
    let token = cancel_token.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = Hub::new(args).run() => {}
            _ = cancel_token.cancelled() => {}
        }

        println!("Shutting hub down");
    });

    println!("Succesfully started hub socket");
    token
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
    file.flush().await.expect("Failed to flush service file");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_dropped_connection() {
    let socket_dir =
        TempDir::new("krossbar_hub_socket_dir").expect("Failed to create socket tempdir");
    let socket_path: String = socket_dir
        .path()
        .join("krossbar_hub.socket")
        .as_os_str()
        .to_str()
        .unwrap()
        .into();

    let cancel_token = start_hub(&socket_path, &get_service_files_dir());
    // Lets wait until hub starts
    time::sleep(Duration::from_millis(10)).await;

    let mut connection = UnixStream::connect(&socket_path)
        .await
        .expect("Failed to connect to the hub");
    println!("Succesfully connected to the hub");

    // Force drop connection
    drop(connection);

    // Try to connect one more time to check if hub is still working
    connection = UnixStream::connect(&socket_path)
        .await
        .expect("Failed to connect to the hub");
    println!("Succesfully connected to the hub second time");
    drop(connection);

    cancel_token.cancel()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_non_existing_service_name() {
    let socket_dir =
        TempDir::new("krossbar_hub_socket_dir").expect("Failed to create socket tempdir");
    let socket_path: String = socket_dir
        .path()
        .join("krossbar_hub.socket")
        .as_os_str()
        .to_str()
        .unwrap()
        .into();

    let service_dir =
        TempDir::new("krossbar_test_non_existing_service_name").expect("Failed to create tempdir");

    let cancel_token = start_hub(
        &socket_path,
        service_dir.path().as_os_str().to_str().unwrap(),
    );
    // Lets wait until hub starts
    time::sleep(Duration::from_millis(10)).await;

    match Service::new("non.existing.name").await {
        Ok(_) => panic!("Shouldn't be allowed"),
        Err(err) => {
            println!("Valid connection error: {err:?}")
        }
    }

    cancel_token.cancel()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_non_allowed_service_name() {
    let socket_dir =
        TempDir::new("krossbar_hub_socket_dir").expect("Failed to create socket tempdir");
    let socket_path: String = socket_dir
        .path()
        .join("krossbar_hub.socket")
        .as_os_str()
        .to_str()
        .unwrap()
        .into();

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

    let cancel_token = start_hub(
        &socket_path,
        service_dir.path().as_os_str().to_str().unwrap(),
    );
    // Lets wait until hub starts
    time::sleep(Duration::from_millis(10)).await;

    match Service::new(service_name).await {
        Ok(_) => panic!("Shouldn't be allowed"),
        Err(err) => {
            println!("Valid connection error: {err:?}")
        }
    }

    cancel_token.cancel()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_valid_registration() {
    let socket_dir =
        TempDir::new("krossbar_hub_socket_dir").expect("Failed to create socket tempdir");
    let socket_path: String = socket_dir
        .path()
        .join("krossbar_hub.socket")
        .as_os_str()
        .to_str()
        .unwrap()
        .into();

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

    let service_name = "com.krossbar.service.name";
    write_service_file(service_dir.path(), service_name, service_file_json).await;

    let cancel_token = start_hub(
        &socket_path,
        service_dir.path().as_os_str().to_str().unwrap(),
    );
    // Lets wait until hub starts
    time::sleep(Duration::from_millis(10)).await;

    Service::new(service_name)
        .await
        .expect("Failed to register valid service");

    cancel_token.cancel()
}
