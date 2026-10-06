#!/bin/sh
set -eu
export LANG="${LINUXDROP_LANG:-de_DE.UTF-8}"
export LANGUAGE="${LINUXDROP_LANGUAGE:-de}"
if [ -d "/run/user/$(id -u)" ]; then export XDG_RUNTIME_DIR="/run/user/$(id -u)"; fi
session_file="${XDG_RUNTIME_DIR:-/nonexistent}/linuxdrop-desktop/bus"
if [ -f "$session_file" ]; then
    export DBUS_SESSION_BUS_ADDRESS="$(cat "$session_file")"
    if gdbus introspect --session --dest org.gnome.Shell --object-path /org/gnome/Shell >/dev/null 2>&1; then
        export WAYLAND_DISPLAY=linuxdrop-nested
        export GDK_BACKEND=wayland
        exec /usr/bin/linuxdrop open
    fi
fi
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec sh "$script_dir/standalone-live.sh"
