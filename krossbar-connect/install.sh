#!/bin/bash

self_dir=$(dirname $(realpath "${0}"))

echo -e "\e[32mInstalling bus connect tool\e[0m"

pushd ${self_dir} > /dev/null
cargo build --release

sudo mkdir -p /etc/krossbar/services/
sudo cp -f krossbar.bus.connect.service /etc/krossbar/services/

sudo cp -f ../target/release/krossbar-bus-connect /usr/bin/
popd > /dev/null
