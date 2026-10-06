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
export GDK_BACKEND=x11 GSK_RENDERER=cairo LIBGL_ALWAYS_SOFTWARE=1
export LANG=de_DE.UTF-8 LINUXDROP_REVIEW_SIZE=480x600
exec_status=0
dbus-run-session -- xvfb-run -a "$1" --ignored --test-threads=1 --nocapture || exec_status=$?
exit "$exec_status"
