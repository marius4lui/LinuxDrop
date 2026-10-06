#!/bin/sh
set -eu
# Scope the mock system address to a fresh private session, never the host bus.
exec dbus-run-session -- sh -c 'export DBUS_SYSTEM_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS" LINUXDROP_TEST_PRIVATE_NM=1; exec cargo test -p linuxdrop-network --test nm_lifecycle -- --ignored --nocapture'
