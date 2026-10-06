#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
target=${CARGO_TARGET_DIR:-"$root/target"}
binary="$target/debug/examples/demo_peer"
if [ ! -x "$binary" ]; then
    cargo build --manifest-path "$root/Cargo.toml" -p linuxdrop-localsend --example demo_peer
fi
printf '%s\n' 'Local demo target: auto-accept is enabled on loopback only. Stop with Ctrl+C.'
exec "$binary" --enable-autoaccept "$@"
