# Panel activation from GNOME Overview

A click on the LinuxDrop top-panel item while Overview was open previously
expanded and immediately collapsed the bubble: the visibility guard still saw
Overview. The explicit activation now hides Overview and opens the bubble after
its modal focus is released. Opening directly inside the `hidden` handler lost
keyboard focus; a cancellable idle callback preserves focus on the visible
bubble header.

Repeated activation, Escape/outside dismissal, disabling the panel setting and
session locking retire the pending open. Extension disable removes the idle
source. Overview transitions alone do not open the bubble; initial state stays
closed. No transfer or consent action is invoked by this handoff.

The isolated native test uses a Clutter virtual pointer to click the actual
panel item. Before the fix it failed with
`Panel click from Overview must dismiss Overview and open the bubble`.
After the fix, the native Shell and preferences smoke passed as `linuxdrop` in
WSL `LinuxDrop-Dev`, GNOME 46 / Mutter 46.2:

```sh
LANG=de_DE.UTF-8 LANGUAGE=de LINUXDROP_SMOKE_TEXT_SCALE=1.5 \
  LINUXDROP_SMOKE_MONITOR=800x600 \
  bash extensions/gnome-shell/tests/run-bubble-smoke.sh
```

New assertions verify panel activation from Overview, header focus, ordinary
Overview show/hide without unsolicited opening, cancellation before the deferred
open runs, and a subsequent panel click. Existing transfer, consent, keyboard,
inactivity, recovery and compact-layout assertions also passed.
`LINUXDROP_SMOKE_PASSED`, `LINUXDROP_PREFS_SMOKE_PASSED` and `git diff --check`
all passed. Expected stripped-down WSL service warnings were present.

No visual layout changed and no new screenshot was captured. Lock/settings
cancellation was source-reviewed; actual locked-session interaction, real
screen-reader speech, physical monitor behavior and other Shell versions were
not tested in this pass. No live demo restart, browser automation, app/crate
change or commit was performed.
