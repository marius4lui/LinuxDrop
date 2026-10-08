#!/bin/sh
set -eu
# Stages into DESTDIR for packages; default /usr, never starts a service.
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
dest=${DESTDIR:-}
prefix=${PREFIX:-/usr}
libexec=${LINUXDROP_LIBEXECDIR:-/usr/libexec/linuxdrop}
case "$libexec" in /usr/libexec/linuxdrop|/usr/lib/linuxdrop) ;; *) printf '%s\n' 'Unsupported helper directory' >&2; exit 2;; esac
target=${LINUXDROP_TARGET_DIR:-"${CARGO_TARGET_DIR:-$root/target}/release"}
if [ "$prefix" != /usr ]; then printf '%s\n' 'Native packaging requires PREFIX=/usr' >&2; exit 1; fi
install -Dm755 "$target/linuxdrop" "$dest$prefix/bin/linuxdrop"
install -Dm755 "$target/linuxdropd" "$dest$prefix/bin/linuxdropd"
install -Dm755 "$target/linuxdrop-netd" "$dest$libexec/linuxdrop-netd"
filin=${LINUXDROP_FILIN:-"${CARGO_TARGET_DIR:-$root/vendor/opendrop-rs/target}/release/filin"}
install -Dm755 "$filin" "$dest$libexec/filin"
install -Dm755 "$root/packaging/helpers/p2p-dhcp.py" "$dest$libexec/p2p-dhcp"
install -Dm644 "$root/packaging/dbus/io.github.marius4lui.LinuxDrop.Netd.conf" "$dest$prefix/share/dbus-1/system.d/io.github.marius4lui.LinuxDrop.Netd.conf"
install -Dm644 "$root/packaging/systemd/linuxdropd.service" "$dest$prefix/lib/systemd/user/linuxdropd.service"
install -Dm644 "$root/packaging/systemd/linuxdrop-netd.service" "$dest$prefix/lib/systemd/system/linuxdrop-netd.service"
sed "s|/usr/libexec/linuxdrop/|$libexec/|g" "$root/packaging/systemd/linuxdrop-netd.service" > "$dest$prefix/lib/systemd/system/linuxdrop-netd.service"
install -Dm644 "$root/crates/linuxdrop-ipc/manager1.xml" "$dest$prefix/share/dbus-1/interfaces/io.github.marius4lui.LinuxDrop.Manager1.xml"
install -Dm644 "$root/packaging/dbus/io.github.marius4lui.LinuxDrop.service" "$dest$prefix/share/dbus-1/services/io.github.marius4lui.LinuxDrop.service"
install -Dm644 "$root/packaging/polkit/io.github.marius4lui.LinuxDrop.policy" "$dest$prefix/share/polkit-1/actions/io.github.marius4lui.LinuxDrop.policy"
install -Dm644 "$root/packaging/polkit/50-linuxdrop-netd.rules" "$dest$prefix/share/polkit-1/rules.d/50-linuxdrop-netd.rules"
install -Dm644 "$root/packaging/io.github.marius4lui.LinuxDrop.desktop" "$dest$prefix/share/applications/io.github.marius4lui.LinuxDrop.desktop"
install -Dm644 "$root/packaging/io.github.marius4lui.LinuxDrop.metainfo.xml" "$dest$prefix/share/metainfo/io.github.marius4lui.LinuxDrop.metainfo.xml"
install -Dm644 "$root/integrations/nautilus/linuxdrop.py" "$dest$prefix/share/nautilus-python/extensions/linuxdrop.py"
install -Dm644 "$root/integrations/dolphin/linuxdrop.desktop" "$dest$prefix/share/kio/servicemenus/linuxdrop.desktop"
install -Dm644 "$root/integrations/thunar/linuxdrop.desktop" "$dest$prefix/share/Thunar/sendto/linuxdrop.desktop"
install -Dm644 "$root/integrations/thunar/uca.xml.example" "$dest$prefix/share/doc/linuxdrop/thunar-uca.xml.example"
install -Dm755 "$root/integrations/thunar/install-action.py" "$dest$prefix/bin/linuxdrop-thunar-install"
install -Dm644 "$root/docs/INSTALL.md" "$dest$prefix/share/doc/linuxdrop/INSTALL.md"
install -Dm644 "$root/docs/releases/0.1.0-beta.1.md" "$dest$prefix/share/doc/linuxdrop/RELEASE.md"
install -Dm644 "$root/LICENSE" "$dest$prefix/share/doc/linuxdrop/copyright"
install -Dm644 "$root/vendor/open-quickshare/LICENSE" "$dest$prefix/share/doc/linuxdrop/licenses/open-quickshare-LICENSE"
install -Dm644 "$root/vendor/bluez-async/LICENSE-MIT" "$dest$prefix/share/doc/linuxdrop/licenses/bluez-async-LICENSE-MIT"
install -Dm644 "$root/vendor/bluez-async/LICENSE-APACHE" "$dest$prefix/share/doc/linuxdrop/licenses/bluez-async-LICENSE-APACHE"
install -Dm644 "$root/vendor/bluer/LICENSE" "$dest$prefix/share/doc/linuxdrop/licenses/bluer-LICENSE"
install -Dm644 "$root/vendor/bluer/bluetooth-numbers-database/LICENSE" "$dest$prefix/share/doc/linuxdrop/licenses/bluetooth-numbers-database-LICENSE"
install -Dm644 "$root/vendor/opendrop-rs/LICENSE" "$dest$prefix/share/doc/linuxdrop/licenses/opendrop-rs-LICENSE"
for document in "$root"/vendor/*.md "$root"/docs/adr/*; do
    if [ -f "$document" ]; then install -Dm644 "$document" "$dest$prefix/share/doc/linuxdrop/provenance/$(basename "$document")"; fi
done
revision=$(git -C "$root" rev-parse HEAD 2>/dev/null || printf unknown)
printf 'LinuxDrop source: https://github.com/marius4lui/LinuxDrop\nSource base revision: %s\nLocal builds can include uncommitted changes. The matching source archive distributed alongside this package is authoritative.\nThird-party license texts are in licenses/, and protocol provenance is in provenance/.\n' "$revision" > "$dest$prefix/share/doc/linuxdrop/SOURCE"
if [ -d "$root/extensions/gnome-shell" ]; then
    extension="$dest$prefix/share/gnome-shell/extensions/linuxdrop@marius4lui.github.io"
    install -d "$extension"
    cp -R "$root/extensions/gnome-shell/." "$extension/"
    find "$extension" -type f -exec chmod 644 '{}' \;
    find "$extension" -type d -exec chmod 755 '{}' \;
    if [ -d "$extension/schemas" ]; then glib-compile-schemas "$extension/schemas"; fi
fi
icon=$(find "$root/app/linuxdrop" -name 'io.github.marius4lui.LinuxDrop.svg' -print -quit)
if [ -n "$icon" ]; then install -Dm644 "$icon" "$dest$prefix/share/icons/hicolor/scalable/apps/io.github.marius4lui.LinuxDrop.svg"; fi
