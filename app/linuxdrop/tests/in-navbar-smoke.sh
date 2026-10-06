#!/bin/sh
set -eu
root=/tmp/linuxdrop-navbar-smoke-1001
export XDG_RUNTIME_DIR="$root/runtime" XDG_CONFIG_HOME="$root/config" XDG_DATA_HOME="$root/data"
export DBUS_SESSION_BUS_ADDRESS="$(cat "$root/bus")"
export WAYLAND_DISPLAY=linuxdrop-navbar-smoke GDK_BACKEND=wayland GSK_RENDERER=cairo GSETTINGS_BACKEND=keyfile
exec "$@"
