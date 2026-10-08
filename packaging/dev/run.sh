#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
cargo build --workspace
target=${CARGO_TARGET_DIR:-"$root/target"}/debug
# D-Bus session is shared by this daemon and GUI. Keep hardware helper separately
# installed via the native package; this script never runs the app as root.
if [ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]; then
    exec dbus-run-session -- sh "$0" "$@"
fi
"$target/linuxdropd" &
daemon=$!
trap 'kill "$daemon" 2>/dev/null || true' EXIT HUP INT TERM
"$target/linuxdrop" "$@"
