#!/bin/sh
# Isolated headless smoke session; never reloads the user's live extension.
set -eu
root=/tmp/linuxdrop-navbar-smoke-1001
mkdir -p "$root/runtime" "$root/config/linuxdrop" "$root/data/gnome-shell/extensions"
chmod 700 "$root" "$root/runtime"
export XDG_RUNTIME_DIR="$root/runtime" XDG_CONFIG_HOME="$root/config" XDG_DATA_HOME="$root/data"
export GSETTINGS_BACKEND=keyfile LIBGL_ALWAYS_SOFTWARE=1 GSK_RENDERER=cairo
export XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP=GNOME
export WAYLAND_DISPLAY=linuxdrop-navbar-smoke
uuid=linuxdrop@marius4lui.github.io
mkdir -p "$root/data/gnome-shell/extensions/$uuid"
cp -a extensions/gnome-shell/. "$root/data/gnome-shell/extensions/$uuid/"
glib-compile-schemas "$root/data/gnome-shell/extensions/$uuid/schemas"
printf '%s' '{"localsend":{"enabled":false},"quickshare":{"enabled":false},"airdrop":{"enabled":false}}' > "$root/config/linuxdrop/settings.json"
exec dbus-run-session -- sh -c '
    printf "%s" "$DBUS_SESSION_BUS_ADDRESS" > /tmp/linuxdrop-navbar-smoke-1001/bus
    gsettings set org.gnome.shell enabled-extensions "['"'"'linuxdrop@marius4lui.github.io'"'"']"
    gsettings set org.gnome.desktop.interface enable-hot-corners false
    gsettings set org.gnome.desktop.interface enable-animations false
    gsettings set org.gnome.desktop.session idle-delay 0
    exec gnome-shell --headless --wayland --no-x11 --wayland-display=linuxdrop-navbar-smoke --virtual-monitor=1440x900
'
