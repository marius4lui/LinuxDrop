#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
version=${LINUXDROP_VERSION:-0.1.0}
arch=$(dpkg --print-architecture)
case "$version" in *[!0-9A-Za-z.+:~\-]*) printf '%s\n' 'Invalid package version' >&2; exit 1;; esac
if [ "${LINUXDROP_SKIP_BUILD:-0}" != 1 ]; then
    cargo build --release --locked --workspace
    cargo build --release --locked --manifest-path vendor/opendrop-rs/Cargo.toml -p filin-rs
fi
stage=$(mktemp -d)
trap 'rm -rf -- "$stage"' EXIT HUP INT TERM
DESTDIR="$stage" sh packaging/install.sh
install -d "$stage/DEBIAN" dist
cat > "$stage/DEBIAN/control" <<EOF
Package: linuxdrop
Version: $version
Section: net
Priority: optional
Architecture: $arch
Maintainer: LinuxDrop contributors <noreply@github.com>
Depends: libgtk-4-1 (>= 4.12), libadwaita-1-0 (>= 1.5), libssl3t64, libc6, libgcc-s1, systemd, dbus, policykit-1, iw, iproute2, ethtool, adduser
Recommends: network-manager, bluez, python3-nautilus, gnome-shell-extension-prefs
Homepage: https://github.com/marius4lui/LinuxDrop
Description: Nearby file sharing for Linux
 Native GTK desktop sharing with LocalSend, Quick Share and experimental AirDrop.
EOF
install -m755 packaging/debian/postinst "$stage/DEBIAN/postinst"
install -m755 packaging/debian/prerm "$stage/DEBIAN/prerm"
install -m755 packaging/debian/postrm "$stage/DEBIAN/postrm"
dpkg-deb --root-owner-group --build "$stage" "dist/linuxdrop_${version}_${arch}.deb"
# Ship the actual build inputs, including vendor patches and Cargo.lock. Avoid
# git archive here because local review packages can contain uncommitted work.
tar --exclude='./.git' --exclude='./target' --exclude='./dist' --exclude='./.dev' --exclude='*/target' --exclude='*/__pycache__' -czf "dist/linuxdrop_${version}_source.tar.gz" .

(cd dist && sha256sum "linuxdrop_${version}_${arch}.deb" "linuxdrop_${version}_source.tar.gz" > SHA256SUMS)
