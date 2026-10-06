#!/bin/sh
set -eu
export LANG="${LINUXDROP_LANG:-de_DE.UTF-8}"
export LANGUAGE="${LINUXDROP_LANGUAGE:-de}"
export GDK_BACKEND=wayland
export GSK_RENDERER=cairo
export LIBGL_ALWAYS_SOFTWARE=1
export WAYLAND_DISPLAY=/mnt/wslg/runtime-dir/wayland-0
state="$HOME/.cache/linuxdrop-live"
mkdir -p "$state/runtime"
chmod 700 "$state" "$state/runtime"
export XDG_RUNTIME_DIR="$state/runtime"
script=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)/standalone-live.sh
if [ "${1:-}" != --inside ]; then
    if [ -f "$state/bus" ]; then
        export DBUS_SESSION_BUS_ADDRESS="$(cat "$state/bus")"
        if gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.GetNameOwner io.github.marius4lui.LinuxDrop.App >/dev/null 2>&1; then
            exec /usr/bin/linuxdrop open
        fi
    fi
    exec dbus-run-session -- sh "$script" --inside >"$state/session.log" 2>&1
fi
printf '%s' "$DBUS_SESSION_BUS_ADDRESS" > "$state/bus"
/usr/bin/linuxdropd &
daemon=$!
peer=''
cleanup() { rm -f "$state/bus"; kill "$daemon" ${peer:+"$peer"} 2>/dev/null || true; }
trap cleanup EXIT HUP INT TERM
if [ -x /opt/linuxdrop-target/debug/examples/demo_peer ]; then
    /opt/linuxdrop-target/debug/examples/demo_peer --enable-autoaccept >"$state/peer.log" 2>&1 &
    peer=$!
fi
/usr/bin/linuxdrop open
