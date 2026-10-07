#!/bin/sh
set -eu
exec dbus-run-session -- sh -eu -c '
    export DBUS_SYSTEM_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS" LINUXDROP_TEST_PRIVATE_BLUEZ=1
    exec cargo test -p linuxdrop-airdrop --lib bluetooth::tests --locked -- --ignored --nocapture
'
