#!/bin/sh
# Pass the compiled `cargo test -p linuxdrop --no-run` test executable.
set -eu
runtime=$(mktemp -d /tmp/linuxdrop-native-test.XXXXXX)
cleanup() {
    # The document portal can leave a FUSE mount until the private bus exits.
    if mountpoint -q "$runtime/doc"; then fusermount3 -u "$runtime/doc" || true; fi
    if mountpoint -q "$runtime/gvfs"; then fusermount3 -u "$runtime/gvfs" || true; fi
    rm -rf -- "$runtime"
}
trap cleanup EXIT HUP INT TERM
export XDG_RUNTIME_DIR="$runtime" GSETTINGS_BACKEND=memory
export XDG_CONFIG_HOME="$runtime/config" XDG_DATA_HOME="$runtime/data" XDG_CACHE_HOME="$runtime/cache"
# Keep desktop portals available, but never activate an installed sharing daemon
# when the fixture deliberately releases its bus name to test owner replacement.
mkdir -p "$runtime/services" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME"
for directory in /usr/share/dbus-1/services /usr/local/share/dbus-1/services; do
    for service in "$directory"/*.service; do
        [ -f "$service" ] || continue
        if ! grep -Eq '^Name[[:space:]]*=[[:space:]]*io\.github\.marius4lui\.LinuxDrop[[:space:]]*$' "$service"; then
            cp "$service" "$runtime/services/"
        fi
    done
done
cat > "$runtime/session.conf" <<EOF
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=/tmp</listen>
  <auth>EXTERNAL</auth>
  <servicedir>$runtime/services</servicedir>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
EOF
export GDK_BACKEND=x11 GSK_RENDERER=cairo LIBGL_ALWAYS_SOFTWARE=1
if [ -n "${LINUXDROP_NATIVE_WAYLAND_DISPLAY:-}" ]; then
    # An absolute socket path keeps the private compositor while this runner
    # isolates the application bus, configuration and protocol fixture again.
    test -S "$LINUXDROP_NATIVE_WAYLAND_DISPLAY"
    export GDK_BACKEND=wayland WAYLAND_DISPLAY="$LINUXDROP_NATIVE_WAYLAND_DISPLAY"
fi
export LANG=de_DE.UTF-8 LINUXDROP_REVIEW_SIZE=480x600
exec_status=0
if [ "$GDK_BACKEND" = wayland ]; then
    dbus-run-session --config-file="$runtime/session.conf" -- "$1" --ignored --test-threads=1 --nocapture || exec_status=$?
else
    dbus-run-session --config-file="$runtime/session.conf" -- xvfb-run -a "$1" --ignored --test-threads=1 --nocapture || exec_status=$?
fi
exit "$exec_status"
