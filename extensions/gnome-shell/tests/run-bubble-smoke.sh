#!/usr/bin/env bash
# Run from the repository root as an unprivileged user. No live session is used.
# Optional LINUXDROP_SMOKE_CAPTURE is an absolute PNG path for the verification view.
set -eu
if [ "$(id -u)" = 0 ]; then echo 'Run this isolated test as an unprivileged user.' >&2; exit 1; fi
runroot=$(mktemp -d /tmp/linuxdrop-bubble-smoke.XXXXXX)
export XDG_RUNTIME_DIR="$runroot/runtime" XDG_CONFIG_HOME="$runroot/config" XDG_DATA_HOME="$runroot/data" XDG_CACHE_HOME="$runroot/cache"
export GSETTINGS_BACKEND=keyfile LIBGL_ALWAYS_SOFTWARE=1 GSK_RENDERER=cairo
export XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP=GNOME
export WAYLAND_DISPLAY=linuxdrop-bubble-smoke
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME/gnome-shell/extensions"
chmod 700 "$XDG_RUNTIME_DIR"
extension="$XDG_DATA_HOME/gnome-shell/extensions/linuxdrop@marius4lui.github.io"
mkdir -p "$extension"
cp -a extensions/gnome-shell/. "$extension/"
glib-compile-schemas "$extension/schemas"
python3 - "$extension/extension.js" <<'PY'
from pathlib import Path
import sys
target = Path(sys.argv[1])
source = target.read_text()
marker = '        this._poll = GLib.timeout_add_seconds'
assert source.count(marker) == 1
probe = Path('extensions/gnome-shell/tests/bubble-smoke.js').read_text()
target.write_text(source.replace(marker, probe + '\n' + marker))
PY
export LINUXDROP_SMOKE_ROOT="$runroot"
dbus-run-session -- bash -c '
    set -eu
    python3 app/linuxdrop/tests/shell-review-fixture.py > "$LINUXDROP_SMOKE_ROOT/fixture.log" 2>&1 &
    fixture_pid=$!
    shell_pid=
    trap '\''kill ${shell_pid:-} "$fixture_pid" 2>/dev/null || true'\'' EXIT
    sleep 1
    gsettings set org.gnome.shell enabled-extensions "['\''linuxdrop@marius4lui.github.io'\'']"
    gsettings set org.gnome.desktop.interface enable-animations false
    gsettings set org.gnome.desktop.interface text-scaling-factor "${LINUXDROP_SMOKE_TEXT_SCALE:-1.0}"
    gsettings set org.gnome.desktop.interface enable-hot-corners false
    gsettings set org.gnome.desktop.notifications show-banners false
    gsettings set org.gnome.desktop.session idle-delay 0
    gnome-shell --headless --wayland --no-x11 --wayland-display=linuxdrop-bubble-smoke --virtual-monitor=1440x900 > "$LINUXDROP_SMOKE_ROOT/shell.log" 2>&1 &
    shell_pid=$!
    for attempt in {1..15}; do
        sleep 1
        if grep -q LINUXDROP_SMOKE_FAILED "$LINUXDROP_SMOKE_ROOT/shell.log"; then cat "$LINUXDROP_SMOKE_ROOT/shell.log"; exit 1; fi
        if grep -q LINUXDROP_SMOKE_PASSED "$LINUXDROP_SMOKE_ROOT/shell.log"; then
            grep LINUXDROP_SMOKE_PASSED "$LINUXDROP_SMOKE_ROOT/shell.log"
            extension="$XDG_DATA_HOME/gnome-shell/extensions/linuxdrop@marius4lui.github.io"
            GI_TYPELIB_PATH="/usr/lib/gnome-shell/girepository-1.0${GI_TYPELIB_PATH:+:$GI_TYPELIB_PATH}" LD_LIBRARY_PATH="/usr/lib/gnome-shell${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" gjs -m "$extension/tests/prefs-smoke.js" "$extension"
            exit 0
        fi
    done
    cat "$LINUXDROP_SMOKE_ROOT/shell.log"
    exit 1
'
