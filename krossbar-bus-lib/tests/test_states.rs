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
async fn test_states_subscription(
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
                "incoming_connections": ["com.subscribe_on_state"]
            }
            "#,
    )
    .unwrap();

    let register_service_name = "com.register_state";
    fixture.write_service_file(register_service_name, service_file_json);

    let mut service1 = Service::new(register_service_name, fixture.hub_socket_path())
        .await
        .expect("Failed to register service");

    let mut state = service1
        .register_state::<i32>("state", 0)
        .expect("Failed to register state");

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

    let service_name = "com.subscribe_on_state";
    fixture.write_service_file(service_name, service_file_json);

    let mut service2 = Service::new(service_name, fixture.hub_socket_path())
        .await
        .expect("Failed to register service");

    let peer = service2
        .connect(register_service_name)
        .await
        .expect("Failed to connect to the target service");

    tokio::spawn(service2.run());

    // Invalid state
    let mut non_existing_state = peer
        .subscribe::<u32>("non_existing_state")
        .await
        .expect("Failed to send state subscription");
    non_existing_state
        .next()
        .await
        .unwrap()
        .expect_err("Subscribed to a non-existing state");

    // Invalid param
    let mut bad_types_state = peer
        .subscribe::<String>("state")
        .await
        .expect("Failed to subscribe to the state");

    time::sleep(Duration::from_millis(10)).await;
    bad_types_state
        .next()
        .await
        .unwrap()
        .expect_err("Valid state from int to String");

    // Valid subscriptions
    let mut good_state = peer
        .subscribe::<u32>("state")
        .await
        .expect("Failed to subscribe to the state");

    assert_eq!(
        good_state
            .next()
            .await
            .unwrap()
            .expect("Good state never arrived"),
        0
    );

    state.set(42).await.unwrap();

    assert_eq!(
        good_state
            .next()
            .await
            .unwrap()
            .expect("Good state never arrived"),
        42
    );

    fixture.cancel()
}

#[rstest]
#[awt]
#[tokio::test(flavor = "multi_thread")]
async fn test_state_call(
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
                "incoming_connections": ["com.call_state"]
            }
            "#,
    )
    .unwrap();

    let register_service_name = "com.register_state";
    fixture.write_service_file(register_service_name, service_file_json);

    let mut service1 = Service::new(register_service_name, fixture.hub_socket_path())
        .await
        .expect("Failed to register service");

    service1
        .register_state("state", 42)
        .expect("Failed to register method");

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

    let service_name = "com.call_state";
    fixture.write_service_file(service_name, service_file_json);

    let mut service2 = Service::new(service_name, fixture.hub_socket_path())
        .await
        .expect("Failed to register service");

    let peer = service2
        .connect(register_service_name)
        .await
        .expect("Failed to connect to the target service");

    tokio::spawn(service2.run());

    // Invalid method
    peer.call::<String, String>("non_existing_state", &"invalid_string".into())
        .await
        .expect_err("Invalid method call succeeded");

    // Invalid return
    peer.call::<(), String>("state", &())
        .await
        .expect_err("Invalid param method call succeeded");

    // Valid call
    assert_eq!(
        peer.call::<(), i32>("state", &())
            .await
            .expect("Failed to make a valid call"),
        42
    );

    fixture.cancel()
}
