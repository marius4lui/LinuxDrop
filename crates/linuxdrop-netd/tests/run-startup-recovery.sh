#!/bin/sh
set -eu
binary=${1:?Pass the built linuxdrop-netd executable}
case "$binary" in /*) ;; *) binary="$(realpath "$binary")" ;; esac
script="$(dirname "$0")/startup-recovery.py"
# All mounts and dummy links remain in this child namespace. The Python probe
# verifies both namespace identities before mounting or writing helper paths.
exec unshare --mount --net --propagation private -- python3 "$script" "$binary"
