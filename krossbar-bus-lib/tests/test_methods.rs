use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use krossbar_bus_lib::service::Service;
use rstest::rstest;
use tokio::time;

mod fixture;
use fixture::{make_fixture, Fixture};

#[rstest]
#[awt]
#[tokio::test(flavor = "multi_thread")]
async fn test_methods(
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
                "incoming_connections": ["com.call_method"]
            }
            "#,
    )
    .unwrap();

    let register_service_name = "com.register_method";
    fixture.write_service_file(register_service_name, service_file_json);

    let mut service1 = Service::new(register_service_name, fixture.hub_socket_path())
        .await
        .expect("Failed to register service");

    service1
        .register_method("method", |client_name, value: i32| async move {
            println!("Client name: {client_name}");

            return format!("Hello, {}", value);
        })
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

    let service_name = "com.call_method";
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

    fixture.cancel()
}

#[rstest]
#[awt]
#[tokio::test(flavor = "multi_thread")]
async fn test_message(
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
                "incoming_connections": ["com.call_method"]
            }
            "#,
    )
    .unwrap();

    let register_service_name = "com.register_method";
    fixture.write_service_file(register_service_name, service_file_json);

    let mut service1 = Service::new(register_service_name, fixture.hub_socket_path())
        .await
        .expect("Failed to register service");

    let message_data = Arc::new(Mutex::new(Vec::<i32>::new()));
    let data_receiver = message_data.clone();

    service1
        .register_method("method", move |client_name, value: i32| {
            let cloned = data_receiver.clone();
            async move {
                println!("Client name: {client_name}");

                cloned.lock().unwrap().push(value)
            }
        })
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

    let service_name = "com.call_method";
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
    peer.message("non_existing_method", &42i32)
        .await
        .expect("Ivalid method failed");

    peer.message::<String>("method", &"invalid_string".into())
        .await
        .expect("Ivalid message body failed");

    peer.message("method", &42i32)
        .await
        .expect("Failed to send message");

    peer.message("method", &43i32)
        .await
        .expect("Failed to send message");

    peer.message("method", &44i32)
        .await
        .expect("Failed to send message");

    // Wait for messages to get handled
    time::sleep(Duration::from_millis(10)).await;

    assert_eq!(
        message_data
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect::<Vec<i32>>(),
        vec![42, 43, 44]
    );

    fixture.cancel()
}
