# Diagnostic export and settings completion review

Diagnostic export now permits one operation per app window. Its disabled action
and translated pending tooltip survive settings reconstruction. Failure or
cancellation restores the action. File-dialog errors are shown; user dismissal
is silent. Responses from an obsolete daemon owner cannot open a save dialog.
The existing typed IPC projection allows only redacted diagnostic fields before
the native file chooser and private Gio file write receive the report.

The native GTK regression passed double activation, rebuilt settings while
pending, service failure/retry and a valid late reply after real D-Bus owner
replacement. Existing draft, settings, peer-preference, incoming-consent and
compact-layout cases passed in the same isolated German session. App all-target
Clippy passed. This fixture does not automate successful chooser selection; the
write path is source-reviewed. Daemon redaction has separate real D-Bus/HTTPS
integration evidence.

The current source inventory contains controls for all 47 user-facing settings:

| Section | Fields | Source evidence |
| --- | ---: | --- |
| General | 5 | name, login, theme, language, close behavior |
| Receiving | 8 | directory, opening, limits, per-request choice, collisions, subfolders |
| Visibility | 3 | mode, duration, lock behavior |
| LocalSend | 6 | enable, port, HTTPS, multicast, PIN and PIN requirement |
| Quick Share | 3 | enable, Bluetooth discovery, port |
| AirDrop | 4 | enable, send, receive, Bluetooth wake |
| Hardware | 5 | preferred radio, USB preference/automation, active-link protection, arrival UI |
| Bluetooth | 1 | explicit controller |
| Notifications | 5 | incoming, completed, errors, sound, private contents |
| Transfers | 4 | parallelism, history count/age, bandwidth |
| Network | 2 | interface allowlist, virtual/VPN interfaces |
| Diagnostics | 1 | logging level |

`crates/linuxdrop-ipc/src/settings.rs` is compared with the section definitions
and field binding in `app/linuxdrop/src/settings.rs`. `schema_version` is internal
metadata. Search, dirty drafts, write errors/retry, exact numeric limits and
reset serialization have native regression coverage. This confirms the UI
inventory, not physical acceptance of every hardware-dependent preference.

Device management has favorite, local label, protocol preference, soft-block and
forget controls. Current native tests cover stable protocol identity, same-peer
reselection, pending/failing preference writes and retry; daemon integration
covers persistence. The UI explicitly distinguishes these preferences from
verified identity.

Folder/GVfs assessment follows the binding plan's regular-local-file scope:
remote URIs, directories and symlinks receive individual visible explanations,
while valid and empty files remain usable. The folder message now recommends
creating an archive instead of implying that an unavailable import action exists.
Automatic recursive or remote staging is not silently performed.
