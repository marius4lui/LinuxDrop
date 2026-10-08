# Completed transfer: real folder launch

Verified in a private GNOME 46 Wayland session, German, 800x600. The fixture
creates a simulated received file in an isolated directory whose path and
filename contain spaces. It presents the production completed-transfer UI,
focuses Open Folder, and activates it with a real virtual-keyboard Space event.

The production action calls GIO's default directory handler. Nautilus opens an
actual visible window; its `org.freedesktop.FileManager1.OpenLocations` property
contains the exact encoded parent-directory URI. The probe queries that property
with `NO_AUTO_START`, so the assertion cannot itself launch the file manager.
The Bubble dismisses and retains no action error. There is no stale Cancel
action on the completed transfer.

Nautilus is made the directory handler only in the fixture's private XDG
configuration, and is closed through the same private session bus afterwards.
The received file and all desktop settings are fixture data; the live demo is
not restarted. No actual protocol transfer is claimed by this check.

```sh
LINUXDROP_SMOKE_COMPLETION=1 LANG=de_DE.UTF-8 LANGUAGE=de \
  LINUXDROP_SMOKE_MONITOR=800x600 \
  bash extensions/gnome-shell/tests/run-bubble-smoke.sh
```

The run passed. The same scenario is wired into the Ubuntu native-desktop CI
job; that remote run is not yet confirmed. Combined with the native selection,
completion-focus and failure/retry checks in the keyboard and Shell acceptance
records, this closes the outstanding Notch completion/folder-action item.
