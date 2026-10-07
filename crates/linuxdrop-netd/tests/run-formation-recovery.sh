#!/bin/sh
set -eu
binary=$(realpath "${1:?Pass the built linuxdrop-netd executable}")
exec unshare --mount --net --propagation private -- dbus-run-session -- \
    python3 "$(dirname "$0")/formation-recovery.py" "$binary"
