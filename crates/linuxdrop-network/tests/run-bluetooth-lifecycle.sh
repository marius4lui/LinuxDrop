#!/bin/sh
set -eu
# Never bind mock BlueZ services to the real system bus.
exec dbus-run-session -- sh -eu -c '
    export DBUS_SYSTEM_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS" LINUXDROP_TEST_PRIVATE_BLUEZ=1 PACKET_BLE_ADVERT=header
    CARGO_PROFILE_DEV_DEBUG=0 cargo test --manifest-path vendor/bluez-async/Cargo.toml --lib messagestream --locked
    cargo test -p linuxdrop-network --test bluetooth_lifecycle --locked -- --ignored --nocapture
    PACKET_BLE_SEND=off cargo test -p linuxdrop-quickshare --test bluetooth_release --locked -- --ignored --nocapture
    cargo test -p linuxdrop-quickshare --test receiver_lifetime --locked -- --ignored --nocapture
    cargo test -p linuxdrop-quickshare --test sender_lifetime --locked -- --ignored --nocapture
    cargo test -p linuxdrop-quickshare --test recipient_scan --locked -- --ignored --nocapture
    exec cargo test -p linuxdrop-quickshare --test signal_bounds --locked -- --ignored --nocapture
'
