#!/bin/sh
set -eu
binary=${1:?Pass the built linuxdrop-netd unit test binary}
exec unshare --net -- env LINUXDROP_TEST_PRIVATE_P2P=1 "$binary" --ignored --exact linux::tests::group_owner_serves_dhcp_without_router_or_dns --nocapture
