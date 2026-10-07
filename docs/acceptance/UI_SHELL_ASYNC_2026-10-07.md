# GNOME Shell asynchronous action feedback — 2026-10-07

Scope: a bounded follow-up to `UI_SHELL_ACTIONS_2026-10-07.md`, confined to
the Shell extension and its existing native smoke. The live demo was not
changed or restarted. No browser automation or Cargo build was used.

## Findings and changes

- Accept, reject, cancel and visibility actions previously became available
  immediately after their D-Bus receipt, before the refreshed state arrived.
  Mutations now remain disabled until a snapshot requested after that receipt
  returns. A read already in flight cannot release the pending state; it triggers
  the coalesced follow-up read. An unchanged revision still settles the action.
- A failed follow-up snapshot enters the existing unavailable-service state and
  clears the pending flags. A later successful read restores usable controls.
  Owner replacement continues to invalidate old action and snapshot callbacks.
- Quick Settings now displays the existing translated working/attention labels
  during pending/failed actions. A rejected visibility change returns to the
  authoritative checked state and remains available for retry.

The existing panel-first opening, drag dwell/drop-surface handoff, monitor
pinning, keyboard focus/scrolling, completion navigation and preference
dependencies were reviewed. No further implementation changes in those paths
were justified by this bounded review.

## Verification

The existing isolated GNOME Shell 46 smoke and GTK preferences smoke passed as
the unprivileged `linuxdrop` user in WSL `LinuxDrop-Dev`:

```sh
LANG=de_DE.UTF-8 LANGUAGE=de LINUXDROP_SMOKE_TEXT_SCALE=1.5 \
  LINUXDROP_SMOKE_MONITOR=800x600 \
  bash extensions/gnome-shell/tests/run-bubble-smoke.sh
```

Both `LINUXDROP_SMOKE_PASSED` and `LINUXDROP_PREFS_SMOKE_PASSED` were reported.
Added assertions cover duplicate-consent suppression during the refresh gap,
pre-action versus post-action reads, settlement with unchanged revision,
visibility failure/retry feedback, failed-refresh recovery, and the Quick
Settings pending state. The existing keyboard and constrained-layout assertions
also passed. `git diff --check` passed for the changed files.

The runner uses native actors with controlled D-Bus replies for the new race
cases. No new screenshot or actual screen-reader traversal was performed.
150% text is not fractional monitor scaling. Physical mixed-DPI/hotplug,
installed application drop flows and real-device interoperability retain the
acceptance boundaries documented in the earlier report. Expected stripped-down
WSL calendar/portal/PipeWire service warnings were present.
