#!/bin/sh
# Private network, bus and document portal; never uses the running desktop.
set -eu
if [ "${LINUXDROP_FD_TEST_PRIVATE:-0}" != 1 ]; then
    exec unshare --net env LINUXDROP_FD_TEST_PRIVATE=1 sh "$0" "$@"
fi
test "$(readlink /proc/self/ns/net)" != "$(readlink /proc/1/ns/net)"
ip link set lo up
ip link add ld-fd-test type dummy
ip address add 192.0.2.1/24 dev ld-fd-test
ip link set ld-fd-test up
runtime=$(mktemp -d /tmp/linuxdrop-fd-test.XXXXXX)
trap 'if mountpoint -q "$runtime/doc"; then fusermount3 -u "$runtime/doc" || true; fi; rm -rf -- "$runtime"' EXIT HUP INT TERM
export XDG_RUNTIME_DIR="$runtime" XDG_CONFIG_HOME="$runtime/config" XDG_DATA_HOME="$runtime/data"
dbus-run-session -- python3 tests/integration/fd_portal.py "$1"
