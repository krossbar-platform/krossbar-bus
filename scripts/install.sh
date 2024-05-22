#!/bin/bash

SYSTEMD_DIR="/etc/systemd/system/"
BIN_DIR="/usr/local/bin/"
SERVICE_DIR="/etc/krossbar/services/"

BUID_TYPE=debug

installSystemd() {
    local FILE=$1
    echo "Copying Systemd file: '${FILE}' into '${SYSTEMD_DIR}'"

    sudo cp -fu "${FILE}" "${SYSTEMD_DIR}"
}

installBin() {
    local FILE=$1
    echo "Copying binary: '../target/${BUID_TYPE}/${FILE}' into '${BIN_DIR}'"

    sudo cp -fu "../target/${BUID_TYPE}/${FILE}" "${BIN_DIR}"
}

installService() {
    local FILE=$1
    echo "Copying service file: '${FILE}' into '${SERVICE_DIR}'"

    sudo cp -fu "${FILE}" "${SERVICE_DIR}"
}

for i in "$@"; do
    case $i in
        -s|--service)
            installService $2
            shift
            shift
            ;;
        -d|--systemd)
            installSystemd $2
            shift
            shift
            ;;
        -b|--bin)
            installBin $2
            shift
            shift
            ;;
        --release)
            BUID_TYPE=release
            shift
            ;;
        -*|--*)
            echo "Unknown option $i"
            exit 1
            ;;
        *)
        ;;
    esac
done

sudo mkdir -p "${SERVICE_DIR}"
