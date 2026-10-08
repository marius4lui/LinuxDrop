#!/bin/sh
# A deployed LinuxDrop service must not auto-activate during fake-owner loss tests.
set -eu
test_binary=$(realpath "$1")
test -x "$test_binary"
runtime=$(mktemp -d /tmp/linuxdrop-native-gtk.XXXXXX)
trap 'rm -rf -- "$runtime"' EXIT HUP INT TERM
export XDG_RUNTIME_DIR="$runtime" XDG_CONFIG_HOME="$runtime/config" XDG_DATA_HOME="$runtime/data" XDG_CACHE_HOME="$runtime/cache"
export DBUS_SYSTEM_BUS_ADDRESS=unix:path=/nonexistent GIO_USE_VFS=local GSK_RENDERER=cairo
mkdir -p "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME"
cat > "$runtime/bus.conf" <<'EOF'
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=/tmp</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*"/>
    <allow receive_sender="*"/>
    <allow own="*"/>
  </policy>
</busconfig>
EOF
# No service directories: only the regression's explicitly registered fake exists.
dbus-run-session --config-file="$runtime/bus.conf" -- xvfb-run -a "$test_binary" native_draft_focus_protocol_and_settings_regressions --ignored --nocapture
