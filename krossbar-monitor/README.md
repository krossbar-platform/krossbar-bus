[![Crates.io][crates-badge]][crates-url]
[![MIT licensed][mit-badge]][mit-url]
[![Build Status][actions-badge]][actions-url]

[crates-badge]: https://img.shields.io/crates/v/krossbar-monitor.svg
[crates-url]: https://crates.io/crates/krossbar-monitor
[mit-badge]: https://img.shields.io/badge/license-MIT-blue.svg
[mit-url]: https://github.com/krossbar-platform/krossbar-bus/blob/main/LICENSE
[actions-badge]: https://github.com/krossbar-platform/krossbar-bus/actions/workflows/ci.yml/badge.svg
[actions-url]: https://github.com/krossbar-platform/krossbar-bus/actions/workflows/ci.yml

# krossbar-monitor

### Krossbar bus monitor

Krossbar monitor allows connecting to Krossbar services to monitor message exchange.

**Note**: To be able to use monitor, you need corresponding features, which are enabled bu default:
- `privileged-services` hub feature, which allows using Krossbar tools;
- `monitor` Krossbar bus library feature, which adds monitoring support to a service.

### Usage
```sh
Krossbar bus monitor

Usage: krossbar-monitor [OPTIONS] <TARGET_SERVICE>

Arguments:
  <TARGET_SERVICE>  Service to monitor

Options:
  -l, --log-level <LOG_LEVEL>  Log level: OFF, ERROR, WARN, INFO, DEBUG, TRACE [default: WARN]
  -h, --help                   Print help
  -V, --version                Print version
```

