#!/bin/bash

self_dir=$(dirname $(realpath "${0}"))

echo -e "\e[32mInstalling bus hub\e[0m"

pushd ${self_dir} > /dev/null
cargo build --release

sudo cp -f systemd/karo.bus.hub.service /etc/systemd/system/
sudo cp -f ../target/release/karo-bus-hub /usr/bin/
popd > /dev/null
