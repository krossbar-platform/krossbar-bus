[![Crates.io][crates-badge]][crates-url]
[![MIT licensed][mit-badge]][mit-url]
[![Build Status][actions-badge]][actions-url]

[crates-badge]: https://img.shields.io/crates/v/krossbar-hub.svg
[crates-url]: https://crates.io/crates/krossbar-hub
[mit-badge]: https://img.shields.io/badge/license-MIT-blue.svg
[mit-url]: https://github.com/krossbar-platform/krossbar-bus/blob/main/LICENSE
[actions-badge]: https://github.com/krossbar-platform/krossbar-bus/actions/workflows/ci.yml/badge.svg
[actions-url]: https://github.com/krossbar-platform/krossbar-bus/actions/workflows/ci.yml

# krossbar-hub

### Krossbar bus hub

The binary acts as a hub for service connections. On request it makes a UDS pair and sends corresponding sockets
to each of the peers, which afterward use the pair for communication.
The only known point of rendezvous fot the services is the hub socket.

The hub uses [krossbar_bus_common::DEFAULT_HUB_SOCKET_PATH] for hub socket path and
[krossbar_bus_common::DEFAULT_SERVICE_FILES_DIR] for service files dir by default.
These can be changed using cmd args.

Created socket has 0o666 file permission to allow service connections.

### Service files
During service registration and later for all connection requests the hub uses permission system to check if the service
is allowed to do what it's trying to do.

Service file filename identifies client service name. The file itself contains executable glob for which it's allowed
to register the service, and a list of client names, who are allowed to connect to the service.

Basic service file `com.example.echo.service` may look like the following:
```json
{
"exec": "/data/krossbar/*",
"incoming_connections": ["**"]
}
```

See [lib examples](https://github.com/krossbar-platform/krossbar-bus/tree/main/krossbar-bus-lib/examples) directory for service files examples.

### Building
Build manually or use `cargo install krossbar-bus-hub` to install.

Usage:
```sh
Krossbar bus hub

Usage: krossbar-bus-hub [OPTIONS]
Options:
  -l, --log-level <LOG_LEVEL>
          Log level: OFF, ERROR, WARN, INFO, DEBUG, TRACE [default: TRACE]
  -a, --additional-service-dirs <ADDITIONAL_SERVICE_DIRS>
          Additional service files directories [default: []]
  -s, --socket-path <SOCKET_PATH>
          Hub socket path [default: /var/run/krossbar.bus.socket]
  -h, --help
          Print help
  -V, --version
          Print version
```
