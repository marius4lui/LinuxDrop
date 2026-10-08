# Shell inactivity preference and keyboard continuity

Bounded follow-up to the Shell action, asynchronous feedback and launch-retry
acceptance reports. The bubble still opens explicitly from its top-panel item.

The inactivity timer previously retained its old deadline when the preference
changed, even when set to zero. It also stopped permanently after expiring during
keyboard interaction unless a later pointer-hover change happened to rearm it.
The timer now responds immediately to that preference, pauses while the bubble
has keyboard focus, and starts a fresh interval when focus leaves. Pointer hover
and the native drop surface retain their existing protection from dismissal.

The added regression failed before the fix with
`Disabling inactivity dismissal must cancel an already running timer`.
After the fix, the existing isolated native Shell and preferences smoke passed
in WSL `LinuxDrop-Dev`, as the unprivileged `linuxdrop` user, using GNOME 46
(Mutter 46.2):

```sh
LANG=de_DE.UTF-8 LANGUAGE=de LINUXDROP_SMOKE_TEXT_SCALE=1.5 \
  LINUXDROP_SMOKE_MONITOR=800x600 \
  bash extensions/gnome-shell/tests/run-bubble-smoke.sh
```

Both `LINUXDROP_SMOKE_PASSED` and `LINUXDROP_PREFS_SMOKE_PASSED` were reported.
The regression covers disabling/enabling the preference while open, keyboard
focus held beyond the interval, focus departure and actual timed dismissal.
Existing transfer, focus, narrow-layout and service-recovery checks also passed.
`git diff --check` passed for the changed Shell files. Expected isolated WSL
calendar/portal/PipeWire warnings were present.

No layout changed, so no new screenshot was needed. No real screen-reader,
mixed-monitor or physical peer acceptance is claimed. No live demo restart,
browser automation, Cargo build or commit was performed.
