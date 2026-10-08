# Actual monitor scaling, 2026-10-07

GNOME 46.2 / Mutter Wayland on Ubuntu was exercised in a separate headless
session, with a 1920x1200 virtual monitor. DisplayConfig applied and reported
125%, 150% and 200% in logical layout mode, respectively 1536x960, 1280x800
and 960x600 logical pixels. The desktop text factor stayed at 1.0. These are
actual compositor scales, not a font-size or integer GDK_SCALE substitute.

At each scale the existing Shell scenario passed real panel clicks, keyboard
scrolling/focus, request actions, progress, dismissal and preferences. Added
geometry assertions require the complete Bubble to fit below the panel inside
the selected logical monitor. The installed GTK Wayland drop surface also
passed measured centering, top-offset changes, close and reopen at all scales.
Compositor screenshots at 150% and 200% were inspected for clipping and legibility.

The existing native GTK scenario additionally ran on the same compositor, with
its own isolated application bus and settings. It passed at all three scales,
covering Send, Transfers, Hardware, Settings, incoming review, compact/wide
layouts and error/retry flows. GTK render captures were inspected at 150%, plus
the compact incoming review at 200%. Widget captures use logical coordinates;
the monitor scale itself is independently verified through Mutter.

One 200% run hit an intermittent numeric-row focus assertion; the next run
passed without a product change. The probe retained a widget reference across
an asynchronous settling interval during which the real polling loop can
rebuild Settings. It now resolves the currently displayed row after settling
and still requires actual keyboard focus before editing. No product focus
assertion was removed or relaxed.

Reproduce (unprivileged, repository root):

```sh
LINUXDROP_SMOKE_SCALE=1.5 LINUXDROP_SMOKE_MONITOR=1920x1200 \
LINUXDROP_SMOKE_REAL_DROP=1 \
LINUXDROP_SMOKE_GTK_TEST=/absolute/path/to/compiled/linuxdrop-test-executable \
bash extensions/gnome-shell/tests/run-bubble-smoke.sh
```

The test executable comes from `cargo test --locked -p linuxdrop --no-run`.
The installed drop-surface check and source GTK scenario are separately optional.
CI now checks the three real monitor scales for Shell 46. This does not establish
physical mixed-DPI hotplug, GPU-specific rendering, other GNOME versions or radio
interoperability. The live demonstration session was not changed.
