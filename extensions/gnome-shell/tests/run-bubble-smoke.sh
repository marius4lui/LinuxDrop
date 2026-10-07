#!/usr/bin/env bash
# Run from the repository root as an unprivileged user. No live session is used.
# Optional LINUXDROP_SMOKE_CAPTURE and LINUXDROP_SMOKE_CAPTURE_PROGRESS are PNG paths.
# LINUXDROP_SMOKE_MONITOR can constrain the isolated virtual monitor (default 1440x900).
# LINUXDROP_SMOKE_REAL_DROP=1 also opens the installed GTK drop surface on this
# private display; LINUXDROP_SMOKE_CAPTURE_DROP optionally records it.
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
    logind_pid=
    trap '\''kill ${shell_pid:-} ${logind_pid:-} "$fixture_pid" 2>/dev/null || true'\'' EXIT
    if [ "${LINUXDROP_SMOKE_MOCK_LOGIND:-0}" = 1 ]; then
        export DBUS_SYSTEM_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS" XDG_SESSION_ID=linuxdrop_ci
        python3 -m dbusmock --session -t logind > "$LINUXDROP_SMOKE_ROOT/logind.log" 2>&1 &
        logind_pid=$!
        python3 extensions/gnome-shell/tests/mock-session.py
    fi
    sleep 1
    gsettings set org.gnome.shell enabled-extensions "['\''linuxdrop@marius4lui.github.io'\'']"
    gsettings set org.gnome.desktop.interface enable-animations false
    gsettings set org.gnome.desktop.interface text-scaling-factor "${LINUXDROP_SMOKE_TEXT_SCALE:-1.0}"
    gsettings set org.gnome.desktop.interface enable-hot-corners false
    gsettings set org.gnome.desktop.notifications show-banners false
    gsettings set org.gnome.desktop.session idle-delay 0
    gnome-shell --headless --wayland --no-x11 --wayland-display=linuxdrop-bubble-smoke --virtual-monitor="${LINUXDROP_SMOKE_MONITOR:-1440x900}" > "$LINUXDROP_SMOKE_ROOT/shell.log" 2>&1 &
    shell_pid=$!
    for attempt in {1..15}; do
        sleep 1
        if grep -q LINUXDROP_SMOKE_FAILED "$LINUXDROP_SMOKE_ROOT/shell.log"; then cat "$LINUXDROP_SMOKE_ROOT/shell.log"; exit 1; fi
        if grep -q LINUXDROP_SMOKE_PASSED "$LINUXDROP_SMOKE_ROOT/shell.log"; then
            grep -E "LINUXDROP_(SMOKE|REAL_DROP)_PASSED" "$LINUXDROP_SMOKE_ROOT/shell.log"
            extension="$XDG_DATA_HOME/gnome-shell/extensions/linuxdrop@marius4lui.github.io"
            # Shell keeps private typelibs/libraries in the distro lib directory.
            for library in /usr/lib/gnome-shell /usr/lib64/gnome-shell /usr/lib/*/gnome-shell; do
                [ -d "$library" ] || continue
                export GI_TYPELIB_PATH="$library/girepository-1.0:$library${GI_TYPELIB_PATH:+:$GI_TYPELIB_PATH}"
                export LD_LIBRARY_PATH="$library${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
            done
            gjs -m "$extension/tests/prefs-smoke.js" "$extension"
            exit 0
        fi
    done
    cat "$LINUXDROP_SMOKE_ROOT/shell.log"
    exit 1
'
