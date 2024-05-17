use std::time::{Duration, Instant};

use krossbar_bus_lib::service::Service;
use log::warn;
use rstest::rstest;
use tokio::time;

mod fixture;
use fixture::{make_warn_fixture, Fixture};

// XXX: Don't forget to use release build to test performance:
// cargo test test_performance_multithread --release -- --nocapture

#[rstest]
#[awt]
#[tokio::test(flavor = "multi_thread")]
async fn test_performance_multithread(
    #[from(make_warn_fixture)]
    #[future]
    fixture: Fixture,
) {
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

    let register_service_name = "com.echo_service";
    fixture.write_service_file(register_service_name, service_file_json);

    let mut echo_service = Service::new(register_service_name, fixture.hub_socket_path())
        .await
        .expect("Failed to register service");

    echo_service
        .register_method("echo", |_, value: i32| async move { value })
        .expect("Failed to register method");

    tokio::spawn(echo_service.run());

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
    fixture.write_service_file(service_name, service_file_json);

    let mut caller_service = Service::new(service_name, fixture.hub_socket_path())
        .await
        .expect("Failed to register service");

    let peer = caller_service
        .connect(register_service_name)
        .await
        .expect("Failed to connect to the target service");

    tokio::spawn(caller_service.run());

    const NUM_CALLS: u32 = 30_000;
    let now = Instant::now();
    for _ in 0..NUM_CALLS {
        let _ = peer.call::<u8, u8>("echo", &0u8).await;
    }

    warn!("Made {NUM_CALLS} call in {}ms", now.elapsed().as_millis());

    fixture.cancel()
}

// XXX: Don't forget to use release build to test performance:
// cargo test test_performance_singlethread --release -- --nocapture

#[rstest]
#[awt]
#[tokio::test(flavor = "current_thread")]
async fn test_performance_singlethread(
    #[from(make_warn_fixture)]
    #[future]
    fixture: Fixture,
) {
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

    let register_service_name = "com.echo_service";
    fixture.write_service_file(register_service_name, service_file_json);

    let mut echo_service = Service::new(register_service_name, fixture.hub_socket_path())
        .await
        .expect("Failed to register service");

    echo_service
        .register_method("echo", |_, value: i32| async move { value })
        .expect("Failed to register method");

    tokio::spawn(echo_service.run());

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
    fixture.write_service_file(service_name, service_file_json);

    let mut caller_service = Service::new(service_name, fixture.hub_socket_path())
        .await
        .expect("Failed to register service");

    let peer = caller_service
        .connect(register_service_name)
        .await
        .expect("Failed to connect to the target service");

    tokio::spawn(caller_service.run());

    const NUM_CALLS: u32 = 100_000;
    let now = Instant::now();
    for _ in 0..NUM_CALLS {
        let _ = peer.call::<u8, u8>("echo", &0u8).await;
    }

    warn!("Made {NUM_CALLS} calls in {}ms", now.elapsed().as_millis());

    fixture.cancel()
}
