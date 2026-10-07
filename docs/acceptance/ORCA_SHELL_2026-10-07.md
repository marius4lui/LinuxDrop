# GNOME Notch screen-reader acceptance

Actual Orca 46.1, GNOME Shell/Mutter 46.2, a private Wayland session, German,
800x600 and 150% text. The probe uses a virtual keyboard through Mutter, with
real Enter, Tab, Space and Escape events. The D-Bus devices/transfers are fixture
data; no consent is submitted and no real transfer is cancelled.

This uncovered an actual keyboard bug that direct focus-setting checks missed:
the outer Notch container was focusable. `St.navigate_focus` then focused that
container instead of traversing its actions. The container is now non-focusable;
focus belongs to its header and buttons. The Notch also exposes the native
dialog accessibility role and a LinuxDrop name, so Orca announces its context.

The corrected native run passed:

- Enter opens the focused panel button, whose request status is spoken.
- Tab reaches Previous, Next, code confirmation, rejection, Details, Drop Files
  and Settings. Tab wraps within the visible Notch.
- Orca announces the peer name, filename information and comparison code on
  opening the dialog. The controls have translated spoken names.
- Space on Next selects the other transfer. Its device and 43% progress are
  announced; the progress actor also exposes the progress-bar role.
- Escape closes the Notch without invoking a transfer action.

Run the dedicated scenario as an unprivileged user:

```sh
LINUXDROP_SMOKE_ORCA=1 LANG=de_DE.UTF-8 LANGUAGE=de \
  LINUXDROP_SMOKE_TEXT_SCALE=1.5 LINUXDROP_SMOKE_MONITOR=800x600 \
  bash extensions/gnome-shell/tests/run-bubble-smoke.sh
```

Orca preferences, bus, Shell extension copy and desktop settings are isolated.
The scenario prints the actual speech-generation evidence and is wired into the
Ubuntu native-desktop CI job. No audio-device or Braille-hardware assessment is
claimed. GNOME 46 is the verified screen-reader version; the separate existing
native controls matrix covers other configured Shell versions.
