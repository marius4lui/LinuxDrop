#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
output=${1:?provide absolute output archive path outside source}
case "$output" in /*) ;; *) printf '%s\n' 'Output path must be absolute' >&2; exit 2;; esac
tar -C "$root" --exclude='./.git' --exclude='./target' --exclude='./dist' --exclude='./.dev' --exclude='*/target' --exclude='*/__pycache__' --exclude='./fuzz/corpus' --exclude='./fuzz/artifacts' -czf "$output" .
