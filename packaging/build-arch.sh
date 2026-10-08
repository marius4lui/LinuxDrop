#!/bin/sh
set -eu
if [ "$(id -u)" = 0 ]; then printf '%s\n' 'makepkg must run as a normal build user.' >&2; exit 2; fi
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
stage=$(mktemp -d)
trap 'rm -rf -- "$stage"' EXIT HUP INT TERM
mkdir -p "$root/dist"
sh "$root/packaging/source-archive.sh" "$stage/linuxdrop_0.1.0_source.tar.gz"
digest=$(sha256sum "$stage/linuxdrop_0.1.0_source.tar.gz" | cut -d' ' -f1)
sed "s/sha256sums=('SKIP')/sha256sums=('$digest')/" "$root/packaging/arch/PKGBUILD" > "$stage/PKGBUILD"
if [ "${LINUXDROP_SKIP_BUILD:-0}" = 1 ]; then
    export LINUXDROP_PREBUILT_DIR="${CARGO_TARGET_DIR:-$root/target}/release"
fi
(cd "$stage" && makepkg --noconfirm)
find "$stage" -maxdepth 1 -name '*.pkg.tar.*' -exec cp '{}' "$root/dist/" \;
cp "$stage/linuxdrop_0.1.0_source.tar.gz" "$root/dist/"
