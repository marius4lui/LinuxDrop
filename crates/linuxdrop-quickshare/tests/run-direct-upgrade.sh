#!/bin/sh
set -eu
binary=${1:?Pass the built rqs_lib unit test executable}
exec unshare --net -- env LINUXDROP_TEST_BWU=1 "$binary" --ignored --exact hdl::inbound::bwu_tests::receiver_hosts_direct_group_and_preserves_encrypted_channel_state --nocapture
