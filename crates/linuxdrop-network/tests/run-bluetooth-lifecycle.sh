#!/bin/sh
set -eu
# Never bind mock BlueZ services to the real system bus.
exec dbus-run-session -- sh -eu -c '
    export DBUS_SYSTEM_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS" LINUXDROP_TEST_PRIVATE_BLUEZ=1 PACKET_BLE_ADVERT=header
    cargo test -p linuxdrop-network --test bluetooth_lifecycle --locked -- --ignored --nocapture
    exec cargo test -p linuxdrop-quickshare --test bluetooth_release --locked -- --ignored --nocapture
'
