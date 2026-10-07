#!/bin/sh
set -eu
# The test verifies its namespace before creating any link or starting DHCP.
binary=${1:?Pass the built linuxdrop-netd unit test binary}
exec unshare --net -- env LINUXDROP_TEST_PRIVATE_P2P=1 "$binary" --ignored --exact linux::tests::ipv6_only_p2p_group_is_ready_without_dhcp --nocapture
