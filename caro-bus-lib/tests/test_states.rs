use std::{
    env,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use json::JsonValue;
use log::LevelFilter;
use tempdir::TempDir;
use tokio::{
    fs::OpenOptions,
    io::AsyncWriteExt,
    sync::mpsc::{self, Sender},
    time,
};

use caro_bus_common::HUB_SOCKET_PATH_ENV;
use caro_bus_hub::{args::Args, hub::Hub};
use caro_bus_lib::Bus;

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
async fn test_states() {
    let socket_dir = TempDir::new("caro_hub_socket_dir").expect("Failed to create socket tempdir");
    let socket_path: String = socket_dir
        .path()
        .join("caro_hub.socket")
        .as_os_str()
        .to_str()
        .unwrap()
        .into();

    let service_dir = TempDir::new("test_states").expect("Failed to create tempdir");

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
                "incoming_connections": ["com.watch_state"]
            }
            "#,
    )
    .unwrap();

    let register_service_name = "com.register_state";
    write_service_file(service_dir.path(), register_service_name, service_file_json).await;

    let mut bus1 = Bus::register(register_service_name)
        .await
        .expect("Failed to register service");

    let mut state = bus1
        .register_state::<i32>("state", 42)
        .expect("Failed to register signal");

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

    let service_name = "com.watch_state";
    write_service_file(service_dir.path(), service_name, service_file_json).await;

    let mut bus2 = Bus::register(service_name)
        .await
        .expect("Failed to register service");

    let peer = bus2
        .connect(register_service_name)
        .await
        .expect("Failed to connect to the target service");

    // Value to be set on call back
    let state_value = Arc::new(Mutex::new(0));
    let state_value_clone = state_value.clone();

    // Invalid state
    peer.watch("non_existing_state", |_: i32| async move {
        panic!("This should never happen");
    })
    .await
    .expect_err("Invalid state watch suceeded");

    // Invalid param
    peer.watch("state", |_: String| async move {
        panic!("This should never happen");
    })
    .await
    .expect_err("Got state with invalid type");

    state.set(11);
    time::sleep(Duration::from_millis(10)).await;
    assert_eq!(*state_value.lock().unwrap(), 0);

    // Valid subscriptions
    assert_eq!(
        peer.watch("state", move |value: i32| {
            let state_value = state_value_clone.clone();
            async move {
                *state_value.lock().unwrap() = value;
            }
        })
        .await
        .expect("Failed to watch state"),
        11
    );

    state.set(42);
    time::sleep(Duration::from_millis(10)).await;
    assert_eq!(*state_value.lock().unwrap(), 42);

    shutdown_tx
        .send(())
        .await
        .expect("Failed to send shutdown request to the hub");
}
