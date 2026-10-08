#!/bin/sh
set -eu
exec dbus-run-session -- sh -c 'export DBUS_SYSTEM_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS" LINUXDROP_TEST_PRIVATE_P2P=1; exec cargo test -p linuxdrop-network --lib p2p::host_tests:: -- --ignored --test-threads=1 --nocapture'
