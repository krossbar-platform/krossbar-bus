#!/bin/bash

self_dir=$(dirname $(realpath "${0}"))

echo "Install script dir: ${self_dir}"

pushd ${self_dir}/krossbar-bus-hub/ > /dev/null
bash ./install.sh
popd > /dev/null

pushd ${self_dir}/krossbar-bus-connect/ > /dev/null
bash ./install.sh
popd > /dev/null

pushd ${self_dir}/krossbar-bus-monitor/ > /dev/null
bash ./install.sh
popd > /dev/null
