#!/bin/bash

self_dir=$(dirname $(realpath "${0}"))

echo -e "\e[32mInstalling bus monitor\e[0m"

pushd ${self_dir} > /dev/null
cargo build --release

sudo mkdir -p /etc/karo/services/
sudo cp -f karo.bus.monitor.service /etc/karo/services/

sudo cp -f ../target/release/karo-bus-monitor /usr/bin/
popd > /dev/null
