# GNOME Shell action continuity — 2026-10-07

Scope: `extensions/gnome-shell`, tested in isolated GNOME Shell 46.0 on
LinuxDrop-Dev. The user's live demo was not modified or restarted. No browser
automation, real-device transfer, or interoperability claim is involved.

## Fixed behavior

- Active and completed transfer views retain a compact **Drop files** action and
  a keyboard-focusable Settings gear with a translated accessible name. Opening
  the drop surface still requires the already-open panel bubble and never sends
  files automatically. Idle state uses the same compact settings control.
- Snapshot changes caused by unrelated transfers preserve the focused action for
  the same transfer and state. Replacing the selected request or changing its
  state moves focus to the header; a new request never inherits consent focus.
  Disabled pending mutations still lose action focus. Transfer arrows retain
  their previous repeated-keyboard-navigation behavior.
- External device names, filenames, verification codes and arbitrary errors are
  literal. Known daemon recovery errors use a separate translation dictionary;
  they cannot accidentally translate an external string matching a UI caption.
- The rendered progress fill begins at the track's left edge. The previous native
  bin centered a correctly sized fill, making the graphic misleading.

## Native evidence

The isolated runner passed with English at 100% text on 1440×900 and German at
150% text on 800×600. Both runs reported `LINUXDROP_SMOKE_PASSED` and
`LINUXDROP_PREFS_SMOKE_PASSED`.

The additional assertions cover persistent actions; action focus when unrelated
transfers arrive/leave; consent focus isolation when request identity changes;
literal names/errors; progress width and left origin; and keyboard reachability
of the new settings action in a constrained scroll viewport. Existing coverage
for hidden initial state, explicit opening, verification, chooser navigation,
PIN, pending mutation, launcher retry, offline recovery, owner replacement,
stale consent callbacks and coalesced refresh remains in the same run.

German native render captures, inspected visually:

- [Incoming verification, 800×600 at 150% text](ui/completion/shell-verification-de150-800x600.png)
- [Active transfer, 800×600 at 150% text](ui/completion/shell-progress-de150-800x600.png)

Run from the repository root as an unprivileged Linux user:

```sh
LANG=en_US.UTF-8 LANGUAGE=en bash extensions/gnome-shell/tests/run-bubble-smoke.sh
LANG=de_DE.UTF-8 LANGUAGE=de LINUXDROP_SMOKE_TEXT_SCALE=1.5 \
  LINUXDROP_SMOKE_MONITOR=800x600 \
  bash extensions/gnome-shell/tests/run-bubble-smoke.sh
```

The runner isolates runtime/config/data/cache, D-Bus and the headless Wayland
display. Optional `LINUXDROP_SMOKE_CAPTURE` and
`LINUXDROP_SMOKE_CAPTURE_PROGRESS` save native PNGs. Standard stripped-down WSL
calendar/PipeWire/portal service warnings occurred; no Shell extension JS error
or failed assertion was reported. `git diff --check` passed for the Shell files.

## Remaining acceptance boundaries

Fixtures provide transfer and device data; they are not physical peers. Real
screen-reader traversal, physical mixed-DPI hotplug/suspend behavior, installed
application drop/portal flows and physical protocol interoperability require
their own acceptance. This focused pass does not broaden those claims.
