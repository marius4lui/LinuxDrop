#!/usr/bin/env bash
set -eu
export XDG_RUNTIME_DIR="/tmp/linuxdrop-review-runtime-$(id -u)"
export DBUS_SESSION_BUS_ADDRESS="$(cat "$HOME/.cache/linuxdrop-review/bus")"
export WAYLAND_DISPLAY=linuxdrop-review
export XDG_SESSION_TYPE=wayland
export XDG_CURRENT_DESKTOP=GNOME
export GSK_RENDERER=cairo
export GDK_BACKEND=wayland
exec "$@"
