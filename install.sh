#!/bin/bash

self_dir=$(dirname $(realpath "${0}"))

echo "Install script dir: ${self_dir}"

pushd ${self_dir}/karo-bus-hub/ > /dev/null
bash ./install.sh
popd > /dev/null

pushd ${self_dir}/karo-bus-connect/ > /dev/null
bash ./install.sh
popd > /dev/null

pushd ${self_dir}/karo-bus-monitor/ > /dev/null
bash ./install.sh
popd > /dev/null
