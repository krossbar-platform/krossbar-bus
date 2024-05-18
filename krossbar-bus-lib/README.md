[![Crates.io][crates-badge]][crates-url]
[![MIT licensed][mit-badge]][mit-url]
[![Build Status][actions-badge]][actions-url]

[crates-badge]: https://img.shields.io/crates/v/krossbar-bus-lib.svg
[crates-url]: https://crates.io/crates/krossbar-bus-lib
[mit-badge]: https://img.shields.io/badge/license-MIT-blue.svg
[mit-url]: https://github.com/krossbar-platform/krossbar-bus/blob/main/LICENSE
[actions-badge]: https://github.com/krossbar-platform/krossbar-bus/actions/workflows/ci.yml/badge.svg
[actions-url]: https://github.com/krossbar-platform/krossbar-bus/actions/workflows/ci.yml

# krossbar-bus-lib

### Krossbar bus lib

A library to register and connect Krossbar services

Krossbar services utilize UDS to communicate with each other.
Krossbar hub acts as a point of rendezvous for the services, checking permissions and connecting counterparties.


To register a service call [Service::new]. This makes a call to the hub trying to register a service with a given name.

[Service] exposes two subsets of methods: to act as a message source, or a message consumer:

#### Service
To act as a message source you have a bunch of method to register service endpoints:
- [Service::register_method] to register a method, which can be called by an arbitrary service. Can be used as a one-way message receiver (see [Client::message]);
- [Service::register_signal] to register a signal, to which peers are able to subscribe. Calling [Signal::emit] will broadcast a value to the signal subscribers;
- [Service::register_state] to register a state, which holds a value and can be can be subscribed to, or called to retrieve the value. Calling [State::set] will broadcast a value to the state subscribers.

#### Client
To act as as message consumer, you have to connect to a client using [Service::connect]. As expected, hub needs to check if you are allowed to connect to the client, but after you've connected, there a couple of methods you can use:
- [Client::call] to call a method or retrieve state value;
- [Client::get] get a remote state value;
- [Client::message] send a one-way message to a remote method;
- [Client::subscribe] to subscribe to a signal, or state changes;
- [Client::message] to send-one-way message.

#### Polling
In order to receive incoming connection and messages you need to poll the [Service]. There are two methods to do this:

1. Using [Service::run] if you don't need a service handle anymore. This is much more convenient way. You can spawn a task to just poll the service in a loop:

    ```rust

    use std::path::PathBuf;

    use tokio;

    use krossbar_bus_lib::service::Service;

    async fn example() {
        let mut service = Service::new("com.echo.service", &PathBuf::from("/var/run/krossbar.hub.socket"))
        .await
        .expect("Failed to register service");

        service
            .register_method("method", |client_name, value: i32| async move {
                println!("Client name: {client_name}");

               return format!("Hello, {}", value);
           })
           .expect("Failed to register method");

        tokio::spawn(service.run());
    }
    ```

2. Using [Service::poll] if you still need a service handle to make new connections, or register new endpoints.
    You can use future combinators to poll both: service handle and endpoint handles.

    ```rust

    use std::path::PathBuf;

    use futures::StreamExt;
    use tokio;

    use krossbar_bus_lib::service::Service;

    async fn example() {
        let mut service = Service::new("com.signal.subscriber", &PathBuf::from("/var/run/krossbar.hub.socket"))
            .await
            .expect("Failed to register service");

        let mut client = service.connect("com.signalling.service")
            .await
            .expect("Failed to register to a service");

        let mut subscription = client.subscribe::<String>("signal")
            .await
            .expect("Failed ot subscribe ot a signal");

        tokio::select! {
            value = subscription.next() => {
                println!("Signal data: {value:?}");
            },
            _ = service.poll() => {}
        }
    }
    ```

#### Service files
To be able to register as a service and receive connections, you need to maintain a service file for each of the services.
The file filename is a service name. File content is a JSON, which contains a path to a binary, which is allowed to register the service name, and a list of allowed connections. Both support globs. See [examples](https://github.com/krossbar-platform/krossbar-bus/tree/main/krossbar-bus-lib/examples) for a bunch of examples.

#### Monitor
Having `monitor` feature allows you to use [Krossbar Monitor](https://github.com/krossbar-platform/krossbar-bus/tree/main/krossbar-bus-monitor) to monitor service communication. Refer to the corresponding crate for usage.

#### Inspection
Having `inspection` feature allows you to use [Krossbar Connect tool](https://github.com/krossbar-platform/krossbar-bus/tree/main/krossbar-bus-connect) to inspect service endpoints. Refer to the corresponding crate for usage.

Also, the tool allows you to call or subscribe to a service using CLI.

### Examples
See [examples dir](https://github.com/krossbar-platform/krossbar-bus/tree/main/krossbar-bus-lib/examples) for usage examples

