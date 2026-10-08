#!/bin/sh
set -eu
# No physical interface is changed. The test verifies namespace isolation before
# creating its dummy IPv6 link and uses only an in-memory helper socket pair.
binary=${1:?Pass the built linuxdropd test binary}
exec unshare --net -- env LINUXDROP_TEST_PRIVATE_AWDL=1 "$binary" --ignored --exact helper::tests::actual_airdrop_actor_retires_after_helper_loss_and_reopens_on_reconnect --nocapture
