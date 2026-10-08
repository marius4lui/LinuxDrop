#!/bin/sh
set -eu
binary=${1:?Pass the built linuxdrop-netd executable}
daemon=${2:?Pass the built linuxdropd executable}
case "$binary" in /*) ;; *) binary="$(realpath "$binary")" ;; esac
case "$daemon" in /*) ;; *) daemon="$(realpath "$daemon")" ;; esac
exec unshare --mount --net --propagation private -- dbus-run-session -- \
    python3 "$(dirname "$0")/awdl-watchdog.py" "$binary" "$daemon"
