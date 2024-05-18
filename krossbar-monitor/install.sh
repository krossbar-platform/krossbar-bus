#!/bin/bash

self_dir=$(dirname $(realpath "${0}"))

echo -e "\e[32mInstalling bus monitor\e[0m"

pushd ${self_dir} > /dev/null
cargo build --release

sudo mkdir -p /etc/krossbar/services/
sudo cp -f krossbar.bus.monitor.service /etc/krossbar/services/

sudo cp -f ../target/release/krossbar-bus-monitor /usr/bin/
popd > /dev/null
