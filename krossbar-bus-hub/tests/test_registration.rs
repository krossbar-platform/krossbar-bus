use std::time::Duration;

use krossbar_bus_lib::service::Service;
use rstest::rstest;
use tokio::{net::UnixStream, time};

mod fixture;
use fixture::{make_fixture, Fixture};

#[rstest]
#[awt]
#[tokio::test(flavor = "multi_thread")]
async fn test_dropped_connection(
    #[from(make_fixture)]
    #[future]
    fixture: Fixture,
) {
    time::sleep(Duration::from_millis(10)).await;

    let mut connection = UnixStream::connect(fixture.hub_socket_path())
        .await
        .expect("Failed to connect to the hub");
    println!("Succesfully connected to the hub");

    // Force drop connection
    drop(connection);

    // Try to connect one more time to check if hub is still working
    connection = UnixStream::connect(fixture.hub_socket_path())
        .await
        .expect("Failed to connect to the hub");
    println!("Succesfully connected to the hub second time");
    drop(connection);

    fixture.cancel()
}

#[rstest]
#[awt]
#[tokio::test(flavor = "multi_thread")]
async fn test_non_existing_service_name(
    #[from(make_fixture)]
    #[future]
    fixture: Fixture,
) {
    // Lets wait until hub starts
    time::sleep(Duration::from_millis(10)).await;

    match Service::new("non.existing.name", fixture.hub_socket_path()).await {
        Ok(_) => panic!("Shouldn't be allowed"),
        Err(err) => {
            println!("Valid connection error: {err:?}")
        }
    }

    fixture.cancel()
}

#[rstest]
#[awt]
#[tokio::test(flavor = "multi_thread")]
async fn test_non_allowed_service_name(
    #[from(make_fixture)]
    #[future]
    fixture: Fixture,
) {
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
    fixture.write_service_file(service_name, service_file_json);

    match Service::new(service_name, fixture.hub_socket_path()).await {
        Ok(_) => panic!("Shouldn't be allowed"),
        Err(err) => {
            println!("Valid connection error: {err:?}")
        }
    }

    fixture.cancel()
}

#[rstest]
#[awt]
#[tokio::test(flavor = "multi_thread")]
async fn test_valid_registration(
    #[from(make_fixture)]
    #[future]
    fixture: Fixture,
) {
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
    fixture.write_service_file(service_name, service_file_json);

    Service::new(service_name, fixture.hub_socket_path())
        .await
        .expect("Failed to register valid service");

    fixture.cancel()
}
