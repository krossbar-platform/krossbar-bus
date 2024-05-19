use std::time::Duration;

use krossbar_bus_lib::Service;
use rstest::rstest;

mod fixture;
use fixture::{make_fixture, Fixture};
use tokio::time;

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

    service1
        .register_method("method", |client_name, value: i32| async move {
            println!("Client name: {client_name}");

            return format!("Hello, {}", value);
        })
        .expect("Failed to register method");

    let _ = service1
        .register_signal::<i32>("signal")
        .expect("Failed to register method");

    let _ = service1
        .register_state::<i32>("signal", 42)
        .expect("Failed to register method");

    tokio::spawn(service1.run());

    // let inspection_service =
}
