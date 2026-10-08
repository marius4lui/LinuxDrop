#!/bin/sh
set -eu
# Ubuntu 24.04+ development machine. No radio state is modified.
if [ "$(id -u)" -ne 0 ]; then printf '%s\n' 'Run with sudo in the development VM.' >&2; exit 1; fi
apt-get update
DEBIAN_FRONTEND=noninteractive apt-get install -y build-essential curl ca-certificates pkg-config libgtk-4-dev libadwaita-1-dev libssl-dev libdbus-1-dev libudev-dev libnl-3-dev libnl-genl-3-dev libpcap-dev libev-dev protobuf-compiler git iw iproute2 policykit-1 desktop-file-utils appstream python3-gi python3-nautilus gnome-shell gnome-session gnome-control-center dbus-user-session xwayland
printf '%s\n' 'Install the Rust toolchain for your normal user from https://rustup.rs, then run cargo build --workspace.'
