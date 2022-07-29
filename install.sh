#!/bin/bash

self_dir=$(dirname $(realpath "${0}"))

echo "Install script dir: ${self_dir}"

echo -e "\e[32mInstalling bus hub\e[0m"
pushd ${self_dir}/caro-bus-hub/ > /dev/null
cargo make --makefile Make.toml install
popd > /dev/null
