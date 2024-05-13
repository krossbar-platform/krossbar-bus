use std::{env, path::Path, time::Duration};

use json::JsonValue;
use krossbar_bus_common::HUB_SOCKET_PATH_ENV;
use krossbar_bus_lib::service::Service;
use log::LevelFilter;
use tempdir::TempDir;
use tokio::{fs::OpenOptions, io::AsyncWriteExt, time};

use krossbar_hub_lib::{args::Args, hub::Hub};
use tokio_util::sync::CancellationToken;

async fn start_hub(socket_path: &str, service_files_dir: &str) -> CancellationToken {
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
async fn test_non_existing_service_connections() {
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
        TempDir::new("krossbar_test_non_existing_connection").expect("Failed to create tempdir");

    let cancel_token = start_hub(
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

    let mut bus = Service::new(service_name)
        .await
        .expect("Failed to register service");

    bus.connect("non.existing.connection.target".into())
        .await
        .expect_err("Connected to non-existing service");

    cancel_token.cancel();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_dead_service_connections() {
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
        TempDir::new("krossbar_test_dead_connection").expect("Failed to create tempdir");

    let cancel_token = start_hub(
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

    let mut bus = Service::new(service_name)
        .await
        .expect("Failed to register service");

    bus.connect(dead_service_name.into())
        .await
        .expect_err("Connected to a dead service");

    cancel_token.cancel();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_non_allowed_connections() {
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
        TempDir::new("test_non_allowed_connections").expect("Failed to create tempdir");

    let cancel_token = start_hub(
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

    let _bus2 = Service::new(target_service_name)
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

    let mut bus2 = Service::new(service_name)
        .await
        .expect("Failed to register service");

    bus2.connect(target_service_name.into())
        .await
        .expect_err("Not allowed connection succeeded");

    cancel_token.cancel();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_allowed_connection() {
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
        TempDir::new("test_non_allowed_connections").expect("Failed to create tempdir");

    let cancel_token = start_hub(
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

    let _bus1 = Service::new(target_service_name)
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

    let mut bus2 = Service::new(service_name)
        .await
        .expect("Failed to register service");

    bus2.connect(target_service_name)
        .await
        .expect("Allowed connection failed");

    cancel_token.cancel();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_await_connection() {
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
        TempDir::new("test_non_allowed_connections").expect("Failed to create tempdir");

    let cancel_token = start_hub(
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
        let mut bus2 = Service::new(service_name)
            .await
            .expect("Failed to register service");

        bus2.connect_await(target_service_name)
            .await
            .expect("Allowed connection failed");
    });

    let _bus1 = Service::new(target_service_name)
        .await
        .expect("Failed to register service");

    join.await.expect("Failed to join connection");

    cancel_token.cancel();
}
