#!/bin/sh
set -eu
binary=${1:?Pass the built rqs_lib unit test executable}
exec unshare --net -- env LINUXDROP_TEST_BWU=1 "$binary" --ignored hdl::inbound::bwu_tests:: --test-threads=1 --nocapture
