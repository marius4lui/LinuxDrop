#!/bin/sh
# Requires installed LinuxDrop, file manager, pyatspi, Xvfb, xdotool and openbox.
# Use an isolated package root; no live desktop or real protocol daemon is used.
set -eu
[ "$(id -u)" != 0 ] || { echo 'Run as an unprivileged isolated test user.' >&2; exit 1; }
case "${1:-}" in nautilus|thunar|dolphin) ;; *) echo 'Choose nautilus, thunar or dolphin' >&2; exit 1;; esac
run=$(mktemp -d /tmp/linuxdrop-filemanager.XXXXXX)
export XDG_RUNTIME_DIR="$run/runtime" XDG_CONFIG_HOME="$run/config" XDG_DATA_HOME="$run/data" XDG_CACHE_HOME="$run/cache"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME"
chmod 700 "$XDG_RUNTIME_DIR"
export GTK_A11Y=atspi GDK_BACKEND=x11 GSK_RENDERER=cairo
export QT_QPA_PLATFORM=xcb QT_X11_NO_MITSHM=1 QT_QUICK_BACKEND=software QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1
export LANG=C.UTF-8 LANGUAGE=en XDG_CURRENT_DESKTOP=GNOME
exec dbus-run-session -- xvfb-run -a -s '-screen 0 1280x900x24' sh -c '
    dbus-update-activation-environment DISPLAY XAUTHORITY XDG_RUNTIME_DIR XDG_CONFIG_HOME XDG_DATA_HOME XDG_CACHE_HOME
    python3 app/linuxdrop/tests/shell-review-fixture.py > "$XDG_RUNTIME_DIR/fixture.log" 2>&1 &
    fixture=$!
    trap "kill $fixture 2>/dev/null || true" EXIT
    python3 tests/integration/filemanager_selection.py "$1"
' sh "$1"
