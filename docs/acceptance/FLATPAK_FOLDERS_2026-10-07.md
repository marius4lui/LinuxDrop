# Persistent receiving destinations in the installed Flatpak

## Failure and implementation

The old installed client reproduced the defect: choosing `Receive directory &
space` persisted `/run/user/999/doc/<id>/Receive directory & space` as the daemon's
receiving directory. That value depended on a document export remaining mounted.
The regression asserted the real host path and failed before the implementation.

Both settings and per-request review now resolve chooser grants before using a
destination. The host validates the document ID, the installed client's read and
write permissions, directory type, relative path, and canonical containment.
Parent traversal, mismatched basenames and symlink escapes are rejected. The
persisted value is the host's canonical directory, not a portal mount path.

Ubuntu 24.04's older portal does not expose GetHostPaths. The host service uses
its Info method. It also rejects re-exporting a FUSE directory descriptor;
already exported folders therefore reuse the existing grant directly. A folder
already accessible through the sandbox's own data mount takes the explicit
O_PATH descriptor export route. Neither path grants access to the entire host.

During folder resolution, incoming consent remains disabled. Failed or cancelled
choices preserve the existing destination. Closed/replaced dialogs and stale
service generations cannot apply late results. Folder controls use weak widget
references to avoid retaining removed rows through signal-reference cycles.

## Verified installed journey

The existing installed-client test now additionally checks:

- Actual settings folder selection stores the host directory, including spaces
  and an ampersand. A sandbox-visible application data folder is also accepted.
- Read-only file grants, malformed IDs, wrong basenames, parent traversal and a
  symlink outside the granted directory are rejected; a real child directory works.
- All document grants are deleted. The real document portal and sharing daemon
  are stopped and restarted on the private bus. The saved directory remains valid.
- An actual LocalSend HTTPS request is reviewed and accepted in the installed
  GTK client; its bytes arrive in the persisted host directory after that restart.
- A second request selects a different directory using the real folder chooser.
  Its grant is revoked before consent; the real payload still arrives at the
  selected host directory, and the default remains unchanged.
- No final payload is present before consent. Existing send/link/receipt checks
  remain part of the same bounded scenario.

The test uses a private network/mount/PID namespace, Xvfb/Openbox, the GTK portal,
a dedicated account and the real release daemon. GTK 3's initial Recent view has
no selectable directory; the fixture enters Home before keyboard navigation.
A registered MIME handler actually launches; FileManager1 is a recording service.
These are software integration results, not physical Apple/Android acceptance.

The native GTK regression passed in 17.85 seconds. App/daemon all-target Clippy
with warnings denied and formatting checks passed. Ubuntu review `.16` and its
matching Flatpak/source artifacts carry this work; exact artifact hashes and
packaged-daemon acceptance are recorded in `dist/BUILD_REPORT_0.1.0+review.20261007.16.json`.
The live Ubuntu demo was not replaced or restarted.

## Remaining scope

The optional sandbox still needs a host route for the Notch preferences button,
which currently tries to launch `gnome-extensions` inside the sandbox. Other
full-project requirements remain in the completion ledgers; the goal is active.

Primary references: [document grants and Info](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Documents.html),
[portal API changes](https://github.com/flatpak/xdg-desktop-portal/blob/main/NEWS.md).
