#!/bin/sh
set -eu
if [ "$(id -u)" -eq 0 ]; then printf '%s\n' 'Run GNOME as a normal local user.' >&2; exit 1; fi
# GNOME 46 nested Wayland target on Ubuntu 24.04. The outer desktop stays intact.
export XDG_CURRENT_DESKTOP=GNOME
export XDG_SESSION_TYPE=wayland
export LANG="${LINUXDROP_LANG:-de_DE.UTF-8}"
export LANGUAGE="${LINUXDROP_LANGUAGE:-de}"
export LIBGL_ALWAYS_SOFTWARE=1
export GALLIUM_DRIVER=llvmpipe
export GSK_RENDERER=cairo
export XDG_RUNTIME_DIR="$HOME/.cache/linuxdrop-desktop/runtime"
export XDG_CONFIG_HOME="$HOME/.config/linuxdrop-desktop"
export XDG_DATA_HOME="$HOME/.local/share/linuxdrop-desktop"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME/linuxdrop" "$XDG_DATA_HOME"
chmod 700 "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME"
if [ ! -f "$XDG_CONFIG_HOME/linuxdrop/settings.json" ]; then
    umask 077
    printf '%s\n' '{"general":{"device_name":"LinuxDrop Desktop-Demo"},"localsend":{"port":53327},"quickshare":{"enabled":false}}' > "$XDG_CONFIG_HOME/linuxdrop/settings.json"
fi
script=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)/nested-gnome.sh
session_dir="$XDG_RUNTIME_DIR/linuxdrop-desktop"
mkdir -p "$session_dir"
chmod 700 "$session_dir"
if [ "${1:-}" != --inside ]; then
    if [ -f "$session_dir/bus" ]; then
        export DBUS_SESSION_BUS_ADDRESS="$(cat "$session_dir/bus")"
        if gdbus introspect --session --dest org.gnome.Shell --object-path /org/gnome/Shell >/dev/null 2>&1; then
            export WAYLAND_DISPLAY=linuxdrop-nested
            export GDK_BACKEND=wayland
            exec /usr/bin/linuxdrop open
        fi
    fi
    exec dbus-run-session -- sh "$script" --inside
fi
# Nested Mutter uses the outer X11 window provided by WSLg; its clients use
# the new private Wayland display inside that window.
unset WAYLAND_DISPLAY
export GDK_BACKEND=x11
/usr/bin/linuxdropd &
daemon_pid=$!
peer_pid=''
gnome-shell --nested --wayland --no-x11 --wayland-display=linuxdrop-nested &
shell_pid=$!
cleanup() { rm -f "$session_dir/bus"; kill "$shell_pid" "$daemon_pid" ${peer_pid:+"$peer_pid"} 2>/dev/null || true; }
trap cleanup EXIT HUP INT TERM
ready=0
while [ "$ready" -lt 60 ]; do
    if ! kill -0 "$shell_pid" 2>/dev/null; then wait "$shell_pid"; exit 1; fi
    if [ -S "$XDG_RUNTIME_DIR/linuxdrop-nested" ] && gdbus introspect --session --dest org.gnome.Shell --object-path /org/gnome/Shell >/dev/null 2>&1; then break; fi
    ready=$((ready + 1)); sleep 0.25
done
if [ "$ready" -eq 60 ]; then printf '%s\n' 'GNOME did not become ready.' >&2; exit 1; fi
printf '%s' "$DBUS_SESSION_BUS_ADDRESS" > "$session_dir/bus"
export WAYLAND_DISPLAY=linuxdrop-nested
export GDK_BACKEND=wayland
gnome-extensions enable linuxdrop@marius4lui.github.io
if [ -x /opt/linuxdrop-target/debug/examples/demo_peer ]; then
    /opt/linuxdrop-target/debug/examples/demo_peer --enable-autoaccept --daemon-port 53327 --port 53329 >"$session_dir/peer.log" 2>&1 &
    peer_pid=$!
fi
/usr/bin/linuxdrop open &
gdbus call --session --dest org.gnome.Shell --object-path /org/gnome/Shell --method org.freedesktop.DBus.Properties.Set org.gnome.Shell OverviewActive '<false>' >/dev/null
wait "$shell_pid"
