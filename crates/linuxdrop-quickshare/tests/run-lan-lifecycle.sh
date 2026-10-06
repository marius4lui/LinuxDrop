#!/bin/sh
set -eu
# The test mutates interfaces only after checking the private-namespace marker
# and comparing its network namespace with PID 1. No host interface is touched.
binary=${1:?Pass the built lan_lifecycle test binary}
exec unshare --net -- env LINUXDROP_TEST_PRIVATE_LAN=1 "$binary" --ignored --exact running_engine_follows_real_address_and_link_changes --nocapture
