use std::time::Duration;

use futures::StreamExt;
use krossbar_bus_lib::service::Service;
use rstest::rstest;
use tokio::time;

mod fixture;
use fixture::{make_fixture, Fixture};

#[rstest]
#[awt]
#[tokio::test(flavor = "multi_thread")]
async fn test_signals(
    #[from(make_fixture)]
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
                "incoming_connections": ["com.subscribe_on_signal"]
            }
            "#,
    )
    .unwrap();

    let register_service_name = "com.register_signal";
    fixture.write_service_file(register_service_name, service_file_json);

    let mut service1 = Service::new(register_service_name, fixture.hub_socket_path())
        .await
        .expect("Failed to register service");

    let signal = service1
        .register_signal::<i32>("signal")
        .expect("Failed to register signal");

    tokio::spawn(service1.run());

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

    let service_name = "com.subscribe_on_signal";
    fixture.write_service_file(service_name, service_file_json);

    let mut service2 = Service::new(service_name, fixture.hub_socket_path())
        .await
        .expect("Failed to register service");

    let peer = service2
        .connect(register_service_name)
        .await
        .expect("Failed to connect to the target service");

    tokio::spawn(service2.run());

    // Invalid signal
    let mut non_existing_signal = peer
        .subscribe::<u32>("non_existing_signal")
        .await
        .expect("Failed to send signal subscription");
    non_existing_signal
        .next()
        .await
        .unwrap()
        .expect_err("Subscribed to a non-existing signal");

    // Invalid param
    let mut bad_types_signal = peer
        .subscribe::<String>("signal")
        .await
        .expect("Failed to subscribe to the signal");

    time::sleep(Duration::from_millis(10)).await;
    signal.emit(42).await.unwrap();
    bad_types_signal
        .next()
        .await
        .unwrap()
        .expect_err("Valid signal from int to String");

    // Valid subscriptions
    let mut good_signal = peer
        .subscribe::<u32>("signal")
        .await
        .expect("Failed to subscribe to the signal");

    time::sleep(Duration::from_millis(10)).await;
    signal.emit(42).await.unwrap();

    assert_eq!(
        good_signal
            .next()
            .await
            .unwrap()
            .expect("Good signal never arrived"),
        42,
    );

    fixture.cancel()
}
