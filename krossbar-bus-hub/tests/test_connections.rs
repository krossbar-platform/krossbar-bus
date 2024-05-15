use std::time::Duration;

use krossbar_bus_lib::service::Service;
use rstest::rstest;
use tokio::time;

mod fixture;
use fixture::{make_fixture, Fixture};

#[rstest]
#[awt]
#[tokio::test(flavor = "multi_thread")]
async fn test_non_existing_service_connections(
    #[from(make_fixture)]
    #[future]
    fixture: Fixture,
) {
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
    fixture.write_service_file(service_name, service_file_json);

    let mut bus = Service::new(service_name, fixture.hub_socket_path())
        .await
        .expect("Failed to register service");

    bus.connect("non.existing.connection.target".into())
        .await
        .expect_err("Connected to non-existing service");

    fixture.cancel();
}

#[rstest]
#[awt]
#[tokio::test(flavor = "multi_thread")]
async fn test_dead_service_connections(
    #[from(make_fixture)]
    #[future]
    fixture: Fixture,
) {
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
    fixture.write_service_file(service_name, service_file_json);

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
    fixture.write_service_file(dead_service_name, service_file_json);

    let mut bus = Service::new(service_name, fixture.hub_socket_path())
        .await
        .expect("Failed to register service");

    bus.connect(dead_service_name.into())
        .await
        .expect_err("Connected to a dead service");

    fixture.cancel();
}

#[rstest]
#[awt]
#[tokio::test(flavor = "multi_thread")]
async fn test_non_allowed_connections(
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
                "incoming_connections": []
            }
            "#,
    )
    .unwrap();

    let target_service_name = "non.allowed.connection.target";
    fixture.write_service_file(target_service_name, service_file_json);

    let _bus2 = Service::new(target_service_name, fixture.hub_socket_path())
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
    fixture.write_service_file(service_name, service_file_json);

    let mut bus2 = Service::new(service_name, fixture.hub_socket_path())
        .await
        .expect("Failed to register service");

    bus2.connect(target_service_name.into())
        .await
        .expect_err("Not allowed connection succeeded");

    fixture.cancel();
}

#[rstest]
#[awt]
#[tokio::test(flavor = "multi_thread")]
async fn test_allowed_connection(
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
                "incoming_connections": ["allowed.connection.initiator"]
            }
            "#,
    )
    .unwrap();

    let target_service_name = "allowed.connection.target";
    fixture.write_service_file(target_service_name, service_file_json);

    let _bus1 = Service::new(target_service_name, fixture.hub_socket_path())
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
    fixture.write_service_file(service_name, service_file_json);

    let mut bus2 = Service::new(service_name, fixture.hub_socket_path())
        .await
        .expect("Failed to register service");

    bus2.connect(target_service_name)
        .await
        .expect("Allowed connection failed");

    fixture.cancel();
}

#[rstest]
#[awt]
#[tokio::test(flavor = "multi_thread")]
async fn test_await_connection(
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
                "incoming_connections": ["allowed.connection.initiator"]
            }
            "#,
    )
    .unwrap();

    let target_service_name = "allowed.connection.target";
    fixture.write_service_file(target_service_name, service_file_json);

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
    fixture.write_service_file(service_name, service_file_json);

    let socket_path = fixture.hub_socket_path().clone();
    let join = tokio::spawn(async move {
        let mut bus2 = Service::new(service_name, &socket_path)
            .await
            .expect("Failed to register service");

        bus2.connect_await(target_service_name)
            .await
            .expect("Allowed connection failed");
    });

    let _bus1 = Service::new(target_service_name, fixture.hub_socket_path())
        .await
        .expect("Failed to register service");

    join.await.expect("Failed to join connection");

    fixture.cancel();
}
