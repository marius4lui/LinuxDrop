#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
stage=$(mktemp -d)
trap 'rm -rf -- "$stage"' EXIT HUP INT TERM
mkdir -p "$stage/SOURCES" "$stage/SPECS" "$root/dist"
sh "$root/packaging/source-archive.sh" "$stage/SOURCES/linuxdrop_0.1.0_source.tar.gz"
cp "$root/packaging/rpm/linuxdrop.spec" "$stage/SPECS/"
if [ "${LINUXDROP_SKIP_BUILD:-0}" = 1 ]; then
    target=${CARGO_TARGET_DIR:-"$root/target"}/release
    tar -czf "$stage/SOURCES/linuxdrop-binaries.tar.gz" -C "$target" linuxdrop linuxdropd linuxdrop-netd filin
    rpmbuild -bb --with prebuilt --define "_topdir $stage" --define 'debug_package %{nil}' "$stage/SPECS/linuxdrop.spec"
else
    rpmbuild -bb --define "_topdir $stage" --define 'debug_package %{nil}' "$stage/SPECS/linuxdrop.spec"
fi
find "$stage/RPMS" -name '*.rpm' -exec cp '{}' "$root/dist/" \;
cp "$stage/SOURCES/linuxdrop_0.1.0_source.tar.gz" "$root/dist/"
