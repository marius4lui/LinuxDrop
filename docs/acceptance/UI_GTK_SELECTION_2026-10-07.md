# GTK selection feedback - 2026-10-07

This bounded native GTK review found three remaining interaction defects after
reviewing the existing completion and journey acceptance records:

- Reactivating the selected recipient reset an explicit protocol choice to the
  device preference. It now preserves the current choice; choosing a different
  recipient still applies that recipient's preference.
- Hardware offered the same preference action before and after a successful
  save. The confirmed radio or Bluetooth preference now has a check icon and the
  translated `Preferred adapter` title and accessible name. Failed writes retain
  the previous state. Settings-only changes refresh this indication, and clicking
  the already preferred adapter does not repeat the write. Successful generic
  settings writes invalidate snapshots started before the save receipt.
- The compact German backend status `Fehler` wrapped into a narrow vertical
  column. Short backend status badges now remain on one line.

The existing isolated native scenario was extended with same-recipient protocol
selection, failed adapter preference and retry, confirmed preference rendering,
keyboard focus, repeated activation, Bluetooth settings-only updates, and the
non-wrapping status badge. It uses a private D-Bus fixture, Xvfb, isolated
runtime/config/data/cache directories, and no installed LinuxDrop activation.

Validation: app and daemon all-target Clippy passed with `-D warnings` after the
selection fixes. After the additional badge correction, app test compilation and
the final isolated native scenario passed (1/1, 19.55 seconds). `git diff --check`
also passed. The scenario raised no RefCell borrow failures. Portal/PipeWire and
private-bus shutdown messages were environmental.

The final German 480x600 captures were inspected: the
[backend status](ui/gtk-selection/hardware-inspection.png) stays on one line, and
the [scrolled adapter preference](ui/gtk-selection/hardware-preferred-adapter.png)
shows the readable `Bevorzugter Adapter` row and check icon. Existing hardware
descriptions still contain untranslated English; this bounded change reuses the
existing translated preferred-adapter string and does not claim a complete
translation audit.

This work does not claim physical adapter switching, screen-reader speech,
mixed-DPI monitors, or installed-session hardware acceptance. No browser was
used and the live demo was not restarted.
