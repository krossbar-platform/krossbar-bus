use std::{env, path::Path, time::Duration};

use json::JsonValue;
use krossbar_bus_common::HUB_SOCKET_PATH_ENV;
use log::LevelFilter;
use tempdir::TempDir;
use tokio::{
    fs::OpenOptions,
    io::AsyncWriteExt,
    sync::mpsc::{self, Sender},
    time,
};

use krossbar_bus_hub::{args::Args, hub::Hub};
use krossbar_bus_lib::Bus;

async fn start_hub(socket_path: &str, service_files_dir: &str) -> Sender<()> {
    env::set_var(HUB_SOCKET_PATH_ENV, socket_path);

    let args = Args {
        log_level: LevelFilter::Debug,
        service_files_dir: service_files_dir.into(),
    };

    // let _ = pretty_env_logger::formatted_builder()
    //     .filter_level(args.log_level)
    //     .try_init();

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
    file.flush().await.expect("Failed to flush service file");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_non_existing_service_connections() {
    let socket_dir = TempDir::new("krossbar_hub_socket_dir").expect("Failed to create socket tempdir");
    let socket_path: String = socket_dir
        .path()
        .join("krossbar_hub.socket")
        .as_os_str()
        .to_str()
        .unwrap()
        .into();

    let service_dir =
        TempDir::new("krossbar_test_non_existing_connection").expect("Failed to create tempdir");

    let shutdown_tx = start_hub(
        &socket_path,
        service_dir.path().as_os_str().to_str().unwrap(),
    )
    .await;
    // Lets wait until hub starts
    time::sleep(Duration::from_millis(10)).await;

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

    let service_name = "non.existing.connection.initiator";
    write_service_file(service_dir.path(), service_name, service_file_json).await;

    let mut bus = Bus::register(service_name)
        .await
        .expect("Failed to register service");

    bus.connect("non.existing.connection.target".into())
        .await
        .expect_err("Connected to non-existing service");

    shutdown_tx
        .send(())
        .await
        .expect("Failed to send shutdown request to the hub");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_dead_service_connections() {
    let socket_dir = TempDir::new("krossbar_hub_socket_dir").expect("Failed to create socket tempdir");
    let socket_path: String = socket_dir
        .path()
        .join("krossbar_hub.socket")
        .as_os_str()
        .to_str()
        .unwrap()
        .into();

    let service_dir = TempDir::new("krossbar_test_dead_connection").expect("Failed to create tempdir");

    let shutdown_tx = start_hub(
        &socket_path,
        service_dir.path().as_os_str().to_str().unwrap(),
    )
    .await;
    // Lets wait until hub starts
    time::sleep(Duration::from_millis(10)).await;

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

    let service_name = "dead.connection.initiator";
    write_service_file(service_dir.path(), service_name, service_file_json).await;

    // Create service file second
    let service_file_json = json::parse(
        r#"
            {
                "exec": "/**/*",
                "incoming_connections": ["**"]
            }
            "#,
    )
    .unwrap();

    let dead_service_name = "dead.connection.target";
    write_service_file(service_dir.path(), dead_service_name, service_file_json).await;

    let mut bus = Bus::register(service_name)
        .await
        .expect("Failed to register service");

    bus.connect(dead_service_name.into())
        .await
        .expect_err("Connected to a dead service");

    shutdown_tx
        .send(())
        .await
        .expect("Failed to send shutdown request to the hub");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_non_allowed_connections() {
    let socket_dir = TempDir::new("krossbar_hub_socket_dir").expect("Failed to create socket tempdir");
    let socket_path: String = socket_dir
        .path()
        .join("krossbar_hub.socket")
        .as_os_str()
        .to_str()
        .unwrap()
        .into();

    let service_dir =
        TempDir::new("test_non_allowed_connections").expect("Failed to create tempdir");

    let shutdown_tx = start_hub(
        &socket_path,
        service_dir.path().as_os_str().to_str().unwrap(),
    )
    .await;
    // Lets wait until hub starts
    time::sleep(Duration::from_millis(10)).await;

    // Create service file second
    let service_file_json = json::parse(
        r#"
            {
                "exec": "/**/*",
                "incoming_connections": []
            }
            "#,
    )
    .unwrap();

    let target_service_name = "non.allowed.connection.target";
    write_service_file(service_dir.path(), target_service_name, service_file_json).await;

    let _bus2 = Bus::register(target_service_name)
        .await
        .expect("Failed to register service");

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

    let service_name = "non.allowed.connection.initiator";
    write_service_file(service_dir.path(), service_name, service_file_json).await;

    let mut bus2 = Bus::register(service_name)
        .await
        .expect("Failed to register service");

    bus2.connect(target_service_name.into())
        .await
        .expect_err("Not allowed connection succeeded");

    shutdown_tx
        .send(())
        .await
        .expect("Failed to send shutdown request to the hub");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_allowed_connection() {
    let socket_dir = TempDir::new("krossbar_hub_socket_dir").expect("Failed to create socket tempdir");
    let socket_path: String = socket_dir
        .path()
        .join("krossbar_hub.socket")
        .as_os_str()
        .to_str()
        .unwrap()
        .into();

    let service_dir =
        TempDir::new("test_non_allowed_connections").expect("Failed to create tempdir");

    let shutdown_tx = start_hub(
        &socket_path,
        service_dir.path().as_os_str().to_str().unwrap(),
    )
    .await;
    // Lets wait until hub starts
    time::sleep(Duration::from_millis(10)).await;

    // Create service file second
    let service_file_json = json::parse(
        r#"
            {
                "exec": "/**/*",
                "incoming_connections": ["allowed.connection.initiator"]
            }
            "#,
    )
    .unwrap();

    let target_service_name = "allowed.connection.target";
    write_service_file(service_dir.path(), target_service_name, service_file_json).await;

    let _bus1 = Bus::register(target_service_name)
        .await
        .expect("Failed to register service");

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

    let service_name = "allowed.connection.initiator";
    write_service_file(service_dir.path(), service_name, service_file_json).await;

    let mut bus2 = Bus::register(service_name)
        .await
        .expect("Failed to register service");

    bus2.connect(target_service_name)
        .await
        .expect("Allowed connection failed");

    shutdown_tx
        .send(())
        .await
        .expect("Failed to send shutdown request to the hub");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_await_connection() {
    let socket_dir = TempDir::new("krossbar_hub_socket_dir").expect("Failed to create socket tempdir");
    let socket_path: String = socket_dir
        .path()
        .join("krossbar_hub.socket")
        .as_os_str()
        .to_str()
        .unwrap()
        .into();

    let service_dir =
        TempDir::new("test_non_allowed_connections").expect("Failed to create tempdir");

    let shutdown_tx = start_hub(
        &socket_path,
        service_dir.path().as_os_str().to_str().unwrap(),
    )
    .await;
    // Lets wait until hub starts
    time::sleep(Duration::from_millis(10)).await;

    // Create service file second
    let service_file_json = json::parse(
        r#"
            {
                "exec": "/**/*",
                "incoming_connections": ["allowed.connection.initiator"]
            }
            "#,
    )
    .unwrap();

    let target_service_name = "allowed.connection.target";
    write_service_file(service_dir.path(), target_service_name, service_file_json).await;

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

    let service_name = "allowed.connection.initiator";
    write_service_file(service_dir.path(), service_name, service_file_json).await;

    let join = tokio::spawn(async move {
        let mut bus2 = Bus::register(service_name)
            .await
            .expect("Failed to register service");

        bus2.connect_await(target_service_name)
            .await
            .expect("Allowed connection failed");
    });

    let _bus1 = Bus::register(target_service_name)
        .await
        .expect("Failed to register service");

    join.await.expect("Failed to join connection");

    shutdown_tx
        .send(())
        .await
        .expect("Failed to send shutdown request to the hub");
}
