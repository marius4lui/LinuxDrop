#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
target=${CARGO_TARGET_DIR:-"$root/target"}/release
if [ "${LINUXDROP_SKIP_BUILD:-0}" != 1 ]; then
    (cd "$root" && cargo build --release --locked -p linuxdrop)
fi
stage=$(mktemp -d)
trap 'rm -rf -- "$stage"' EXIT HUP INT TERM
cp "$target/linuxdrop" "$stage/linuxdrop"
cp "$root/packaging/flatpak/io.github.marius4lui.LinuxDrop.App.json" "$stage/manifest.json"
cp "$root/app/linuxdrop/resources/io.github.marius4lui.LinuxDrop.svg" "$stage/io.github.marius4lui.LinuxDrop.App.svg"
sed 's/^Icon=.*/Icon=io.github.marius4lui.LinuxDrop.App/' "$root/packaging/io.github.marius4lui.LinuxDrop.desktop" > "$stage/io.github.marius4lui.LinuxDrop.App.desktop"
mkdir -p "$root/dist"
flatpak-builder --user --force-clean --repo="$stage/repo" "$stage/build" "$stage/manifest.json"
flatpak build-bundle "$stage/repo" "$root/dist/linuxdrop-client.flatpak" io.github.marius4lui.LinuxDrop.App
printf '%s\n' 'Client bundle ready. Install the native LinuxDrop daemon package on the host first.'
