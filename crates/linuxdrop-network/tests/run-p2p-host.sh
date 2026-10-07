#!/bin/sh
set -eu
exec dbus-run-session -- sh -c 'export DBUS_SYSTEM_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS" LINUXDROP_TEST_PRIVATE_P2P=1; exec cargo test -p linuxdrop-network --lib autonomous_group_credentials_cleanup_and_cancellation_race -- --ignored --nocapture'
