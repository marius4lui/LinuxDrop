# Shell launch retry recovery — 2026-10-07

Bounded follow-up to the Shell actions and asynchronous-action reports. Reviewed
panel-only opening, dismissal/focus, drop handoff, transfer feedback and preference
behavior. One remaining recovery defect justified a change: after an app launch
failed, a successful retry left its old error visible in the reopened bubble and
kept Quick Settings at “Needs attention”. An accepted launch now clears that
transient action error. A later drop-window timeout still reports a fresh error;
transfer failures remain attached to their transfer data.

The added existing-harness regression failed before the fix with
`Successful launch retry must retire its old error when the bubble reopens`.
After the fix, the isolated native GNOME Shell and preferences smoke passed as
unprivileged `linuxdrop` in WSL `LinuxDrop-Dev`:

```sh
LANG=de_DE.UTF-8 LANGUAGE=de LINUXDROP_SMOKE_TEXT_SCALE=1.5 \
  LINUXDROP_SMOKE_MONITOR=800x600 \
  bash extensions/gnome-shell/tests/run-bubble-smoke.sh
```

Both `LINUXDROP_SMOKE_PASSED` and `LINUXDROP_PREFS_SMOKE_PASSED` were reported;
`git diff --check` passed. Existing narrow-layout, keyboard, progress and daemon
recovery assertions also passed. Expected isolated WSL service warnings occurred.

The launcher outcome is controlled in this regression; it verifies error-state
recovery, not an installed application's startup. No new rendered screenshot,
screen-reader traversal, physical peer or mixed-DPI acceptance is claimed. No
live demo, GTK application, daemon or packaging file was modified by this pass;
no browser, Cargo build, or commit was used.
