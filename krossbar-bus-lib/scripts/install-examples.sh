#!/bin/bash

# set -x

SCRIPTPATH="$( cd -- "$(dirname "$0")" >/dev/null 2>&1 ; pwd -P )"
BUILD_TYPE="debug"

copyExamplesBinaries() {
    for FILE in ${SCRIPTPATH}/../examples/*.rs; do
        local FILENAME=$(basename "${FILE}")
        FILENAME="${FILENAME%.*}"

        sudo cp -v ${SCRIPTPATH}/../../target/${BUILD_TYPE}/examples/${FILENAME} /usr/local/bin
    done
}

if [ "$1" == "release" ]; then
    BUILD_TYPE="release"
fi

sudo cp -v ${SCRIPTPATH}/../examples/*.service /etc/krossbar/services/
copyExamplesBinaries