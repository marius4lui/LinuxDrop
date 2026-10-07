#!/bin/sh
# Usage: sudo sh tests/integration/run-flatpak-client.sh TEST_USER /absolute/linuxdropd
# TEST_USER must be a disposable account with the client and its runtime installed.
# No installation, real desktop session, network or radio settings are changed.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if [ "${1:-}" = --session ]; then
    shift
    test "${LINUXDROP_FP_PRIVATE:-0}" = 1
    test "$(id -u)" != 0
    runtime="/run/user/$(id -u)"
    export XDG_RUNTIME_DIR="$runtime" XDG_CONFIG_HOME="$runtime/config"
    export XDG_DATA_HOME="$runtime/data" XDG_CACHE_HOME="$runtime/cache"
    export FLATPAK_USER_DIR="$HOME/.local/share/flatpak"
    export XDG_DATA_DIRS="$FLATPAK_USER_DIR/exports/share:/usr/local/share:/usr/share"
    export GTK_A11Y=atspi GDK_BACKEND=x11 GSK_RENDERER=cairo LANG=C.UTF-8 LANGUAGE=en
    export XDG_CURRENT_DESKTOP=linuxdrop-test
    mkdir -p "$XDG_CONFIG_HOME/xdg-desktop-portal" "$XDG_DATA_HOME" "$XDG_CACHE_HOME"
    printf '[preferred]\ndefault=gtk\n' > "$XDG_CONFIG_HOME/xdg-desktop-portal/portals.conf"
    exec dbus-run-session -- xvfb-run -a -s '-screen 0 1280x900x24' sh -c '
        dbus-update-activation-environment DISPLAY XAUTHORITY XDG_RUNTIME_DIR XDG_CONFIG_HOME XDG_DATA_HOME XDG_CACHE_HOME XDG_CURRENT_DESKTOP XDG_DATA_DIRS
        openbox > "$XDG_RUNTIME_DIR/openbox.log" 2>&1 &
        exec timeout 100s python3 "$1/flatpak_client.py" "$2"
    ' sh "$here" "$1"
fi
test "$(id -u)" = 0
if [ "$#" != 2 ]; then
    printf '%s\n' 'Usage: run-flatpak-client.sh TEST_USER /absolute/linuxdropd' >&2
    exit 2
fi
user=$1
daemon=$(realpath "$2")
test -x "$daemon"
test "$(id -u "$user")" != 0
if [ "${LINUXDROP_FP_PRIVATE:-0}" != 1 ]; then
    exec unshare --mount --pid --fork --kill-child --net --mount-proc env \
        LINUXDROP_FP_PRIVATE=1 LINUXDROP_FP_PARENT_NET="$(readlink /proc/self/ns/net)" \
        sh "$0" "$user" "$daemon"
fi
test "$(readlink /proc/self/ns/net)" != "$LINUXDROP_FP_PARENT_NET"
mount --make-rprivate /
# Standard runtime location is essential: Flatpak remounts portals here.
mount -t tmpfs tmpfs /run/user
uid=$(id -u "$user")
mkdir -p "/run/user/$uid" /run/user/linuxdrop-x11
chown "$uid:$(id -g "$user")" "/run/user/$uid"
chmod 700 "/run/user/$uid"
chmod 1777 /run/user/linuxdrop-x11
# WSLg's shared X11 mount must remain untouched. Xvfb needs a pathname socket.
mount --bind /run/user/linuxdrop-x11 /tmp/.X11-unix
ip link set lo up
ip link add ld-fp-test type veth peer name ld-fp-peer
ip address add 198.18.0.1/24 dev ld-fp-test
ip link set ld-fp-test up
ip link set ld-fp-peer up
exec runuser -u "$user" -- sh "$0" --session "$daemon"
