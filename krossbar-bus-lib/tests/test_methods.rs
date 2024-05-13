use std::{env, path::Path, time::Duration};

use json::JsonValue;
use krossbar_bus_lib::service::Service;
use krossbar_hub_lib::{args::Args, hub::Hub};
use log::LevelFilter;
use tempdir::TempDir;
use tokio::{fs::OpenOptions, io::AsyncWriteExt, time};

use krossbar_bus_common::HUB_SOCKET_PATH_ENV;
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
async fn test_methods() {
    let socket_dir =
        TempDir::new("krossbar_hub_socket_dir").expect("Failed to create socket tempdir");
    let socket_path: String = socket_dir
        .path()
        .join("krossbar_hub.socket")
        .as_os_str()
        .to_str()
        .unwrap()
        .into();

    let service_dir = TempDir::new("test_method_calls").expect("Failed to create tempdir");

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
                "incoming_connections": ["com.call_method"]
            }
            "#,
    )
    .unwrap();

    let register_service_name = "com.register_method";
    write_service_file(service_dir.path(), register_service_name, service_file_json).await;

    let mut service1 = Service::new(register_service_name)
        .await
        .expect("Failed to register service");

    service1
        .register_method("method", |value: i32| async move {
            return format!("Hello, {}", value);
        })
        .expect("Failed to register method");

    // Create service file first
    let service_file_json = json::parse(
        r#"
        {
            "exec": "/**/*",
            "incoming_connections": []
        }
        "#,
    )
    .unwrap();

    let service_name = "com.call_method";
    write_service_file(service_dir.path(), service_name, service_file_json).await;

    let mut service2 = Service::new(service_name)
        .await
        .expect("Failed to register service");

    let peer = service2
        .connect(register_service_name)
        .await
        .expect("Failed to connect to the target service");

    // Invalid method
    peer.call::<String, String>("non_existing_method", &"invalid_string".into())
        .await
        .expect_err("Invalid method call succeeded");

    // Invalid param
    peer.call::<String, String>("method", &"invalid_string".into())
        .await
        .expect_err("Invalid param method call succeeded");

    // Invalid return
    peer.call::<String, i32>("method", &"invalid_string".into())
        .await
        .expect_err("Invalid return method call succeeded");

    // Valid call
    assert_eq!(
        peer.call::<i32, String>("method", &42)
            .await
            .expect("Failed to make a valid call"),
        "Hello, 42"
    );

    cancel_token.cancel()
}
