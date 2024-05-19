[![Crates.io][crates-badge]][crates-url]
[![MIT licensed][mit-badge]][mit-url]
[![Build Status][actions-badge]][actions-url]

[crates-badge]: https://img.shields.io/crates/v/krossbar-connect.svg
[crates-url]: https://crates.io/crates/krossbar-connect
[mit-badge]: https://img.shields.io/badge/license-MIT-blue.svg
[mit-url]: https://github.com/krossbar-platform/krossbar-bus/blob/main/LICENSE
[actions-badge]: https://github.com/krossbar-platform/krossbar-bus/actions/workflows/ci.yml/badge.svg
[actions-url]: https://github.com/krossbar-platform/krossbar-bus/actions/workflows/ci.yml

# krossbar-connect

### Krossbar bus connect

Krossbar connect allows connecting to Krossbar services to inspect endpoints or make calls.

**Note**: To be able to use connect, you need corresponding features, which are enabled by default:
- `privileged-services` hub feature, which allows using Krossbar tools;
- `inspection` Krossbar bus library feature, which adds `inspect` service endpoint.

### Usage
Running the binary allows you to connect to a service. If succefully connected, the tool enters
interactive mode and provides a set of commands for usage:
- `help` to print commands help
- `inspect` to inspect target service endpoint;
- `call {method_name} {args_json}` to call a method. Args should be a valid JSON and deserialize into the method params type.
- `subscribe {signal_name}` to subscribe to a signal or a state. This spawns an async task, so you can continue working with the service. All incoming signal emmitions will output into stdout.
- `q` to quit the tool.

```bash
Krossbar bus connect tool

Usage: krossbar-connect [OPTIONS] <TARGET_SERVICE>

Arguments:
  <TARGET_SERVICE>  Service to connect to

Options:
  -l, --log-level <LOG_LEVEL>  Log level: OFF, ERROR, WARN, INFO, DEBUG, TRACE [default: WARN]
  -h, --help                   Print help
  -V, --version                Print version
```

