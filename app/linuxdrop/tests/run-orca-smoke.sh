#!/bin/sh
set -eu
binary=${1:?Pass the LinuxDrop application executable}
output=${2:?Pass an output directory for fixture logs}
root=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
task_runtime=$(mktemp -d /tmp/linuxdrop-orca.XXXXXX)
cleanup() {
    for mounted in "$task_runtime/runtime/doc" "$task_runtime/runtime/gvfs"; do
        if mountpoint -q "$mounted"; then fusermount3 -u "$mounted" || true; fi
    done
    rm -rf -- "$task_runtime"
}
trap cleanup EXIT
export HOME="$task_runtime/home" XDG_RUNTIME_DIR="$task_runtime/runtime"
export XDG_CONFIG_HOME="$task_runtime/config" XDG_DATA_HOME="$task_runtime/data" XDG_CACHE_HOME="$task_runtime/cache"
mkdir -p "$HOME" "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME" "$task_runtime/services"
chmod 700 "$XDG_RUNTIME_DIR"
# Permit desktop support services but never activate the installed sharing daemon.
for service in /usr/share/dbus-1/services/*.service; do
    if ! grep -Eq '^Name[[:space:]]*=[[:space:]]*io\.github\.marius4lui\.LinuxDrop[[:space:]]*$' "$service"; then
        cp "$service" "$task_runtime/services/"
    fi
done
cat > "$task_runtime/session.conf" <<EOF
<busconfig>
 <type>session</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth>
 <servicedir>$task_runtime/services</servicedir>
 <policy context="default"><allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/><allow own="*"/></policy>
</busconfig>
EOF
export GTK_A11Y=atspi GDK_BACKEND=x11 GSK_RENDERER=cairo LANG=de_DE.UTF-8 LANGUAGE=de
export LINUXDROP_REVIEW_SIZE=610x760 LINUXDROP_PRIVATE_A11Y=1 LINUXDROP_A11Y_OUTPUT="$output"
dbus-run-session --config-file="$task_runtime/session.conf" -- xvfb-run -a python3 "$root/app/linuxdrop/tests/orca-smoke.py" "$binary"
