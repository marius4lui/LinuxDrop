#!/bin/sh
set -eu
binary=${1:?Pass the built netd unit test executable}
filin=${2:?Pass the built filin executable}
exec unshare --net -- env LINUXDROP_TEST_PRIVATE_AWDL=1 LINUXDROP_TEST_FILIN="$filin" \
    "$binary" --ignored --exact linux::acquire::tests::real_managed_filin_rejects_channel_failure_without_false_readiness --nocapture
