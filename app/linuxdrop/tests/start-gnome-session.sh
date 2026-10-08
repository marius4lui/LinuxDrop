#!/usr/bin/env bash
# Isolated native GNOME/Wayland review session. Run as an unprivileged test user.
set -eu
export XDG_RUNTIME_DIR="/tmp/linuxdrop-review-runtime-$(id -u)"
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
export XDG_SESSION_TYPE=wayland
export XDG_CURRENT_DESKTOP=GNOME
export XDG_SESSION_DESKTOP=gnome
export LIBGL_ALWAYS_SOFTWARE=1
export GSK_RENDERER=cairo
export MUTTER_DEBUG_DUMMY_MODE_SPECS=1440x900
mkdir -p "$HOME/.cache/linuxdrop-review"
exec dbus-run-session -- bash -c '
    export WAYLAND_DISPLAY=linuxdrop-review
    export DISPLAY=:91
    printf "%s" "$DBUS_SESSION_BUS_ADDRESS" > "$HOME/.cache/linuxdrop-review/bus"
    gsettings set org.gnome.desktop.interface color-scheme default
    gsettings set org.gnome.desktop.interface enable-animations false
    gsettings set org.gnome.desktop.interface enable-hot-corners false
    gsettings set org.gnome.desktop.session idle-delay 0
    gnome-shell --headless --wayland --wayland-display=linuxdrop-review --virtual-monitor=1440x900 --no-x11 > "$HOME/.cache/linuxdrop-review/shell.log" 2>&1 &
    shell_pid=$!
    trap "kill $shell_pid 2>/dev/null || true" EXIT
    wait "$shell_pid"
'
