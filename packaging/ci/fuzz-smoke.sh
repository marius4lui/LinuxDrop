#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
for target in localsend_offer airdrop_archive hardware_report; do
    mkdir -p "fuzz/corpus/$target"
    cp "fuzz/seeds/$target/"* "fuzz/corpus/$target/"
    cargo +nightly fuzz run "$target" -- -max_total_time="${LINUXDROP_FUZZ_SECONDS:-30}" -max_len=65536 -rss_limit_mb=1024
done
