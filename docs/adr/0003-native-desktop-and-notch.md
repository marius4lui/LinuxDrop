# Native desktop and file-drop notch

Status: implemented; GNOME 46 / Ubuntu 24.04 baseline.

The GTK4/libadwaita application owns file selection, editable send drafts, peer
selection, transfers, hardware inspection, and searchable settings. The user
daemon owns protocol state and settings. Both the app and GNOME extension use
the same versioned D-Bus manager and JSON snapshots. The app never acquires
network privileges. `io.github.marius4lui.LinuxDrop.App` is deliberately distinct
from the daemon's bus name.

The main window separates Send, Transfers, Hardware, and Settings. A pinned
send bar stays reachable when the file/peer list scrolls. Selecting files never
starts a transfer: the user chooses a peer/protocol or explicitly creates a
temporary browser download offer. Incoming transfers and verification codes
require an explicit accept/reject decision.

## A real file payload at the desktop notch

The Shell extension owns a dedicated LinuxDrop top-panel button, Quick Settings, expanded transfer
cards, monitor placement, fullscreen/overview/lock hiding, and lifecycle. It
does not pretend that Shell drag-end or XDND leave events contain files or prove
a successful drop.

The bubble is completely hidden until the user clicks the panel button. Clicking
the button again, clicking outside, Escape, or the last transfer finishing closes
it. Incoming requests and progress only update the panel icon while closed; they
never open the bubble automatically. Quick Settings visibility remains separate.

After opening the bubble, choosing **Drop files**, or a short dwell with an
external file drag over that open bubble, opens a temporary
undecorated GTK window. The extension identifies it by this app's ID and its
fixed `LinuxDrop Drop Surface` title, waits for its first frame, positions it at
the notch with `Meta.Window.move_resize_frame`, and hides the overlapping Shell
actor. GTK's `Gdk.FileList` drop target then receives the actual payload. Local
file paths join the same send draft used by the main app. Cancelled drags do not
add files. Closing the surface or disabling the extension removes the temporary
surface; it is not a permanently floating application window.

This hybrid is intentional: compositor actors provide desktop placement while
GTK provides a supported native file-drop target. A successful native Nautilus
Wayland drag was tested through this complete handoff, including two selected
files and Escape cancellation. Sandboxed file-manager portals, remote URIs,
fractional scaling and other compositors remain separate acceptance cases.
Remote URIs are rejected rather than interpreted as local paths.

The normal GTK application and daemon remain usable without the extension. The
extension currently declares only GNOME 46, the version actually exercised.
Incoming system notifications originate in the daemon; the passive notch avoids
emitting a duplicate notification. German and English product strings are
selected from the process locale, and app appearance follows the persisted
System/Light/Dark setting.

Primary API references:

- [GTK FileDialog](https://docs.gtk.org/gtk4/class.FileDialog.html)
- [GDK FileList](https://docs.gtk.org/gdk4/struct.FileList.html)
- [GNOME Quick Settings extension guide](https://gjs.guide/extensions/topics/quick-settings.html)
- [GNOME 46 Shell XDND handler](https://gitlab.gnome.org/GNOME/gnome-shell/-/blob/gnome-46/js/ui/xdndHandler.js)
- [GNOME 46 Mutter DND interface](https://gitlab.gnome.org/GNOME/mutter/-/blob/gnome-46/src/meta/meta-dnd.h)

See [native visual acceptance](../acceptance/UI_NATIVE_2026-10-06.md) for measured
results and remaining device/display cases.
