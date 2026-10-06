#!/bin/sh
set -eu
# Never bind the mock BlueZ service to the real system bus.
exec dbus-run-session -- sh -c 'export DBUS_SYSTEM_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS" LINUXDROP_TEST_PRIVATE_BLUEZ=1; exec cargo test -p linuxdrop-network --test bluetooth_lifecycle --locked -- --ignored --nocapture'
