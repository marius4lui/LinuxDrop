# Installed Flatpak client and receipt actions

The actual installed GNOME 50 client was tested with the current release host
daemon in a disposable Ubuntu 24.04 account, private mount/PID/network namespaces,
private D-Bus and Xvfb. Flatpak 1.14.6; host GTK portal and document portal; GNOME
50.5 runtime commit `0bd5091046708e64bc882e8349877cfbae713f6a1e2d0487a5f10af820a2d792`.
The live Ubuntu demo and its installed daemon were left untouched.

## Reproduced and fixed

The source selected by GTK's actual portal chooser reached the daemon through
Unix descriptors. A completed host receipt, however, failed with "Failed to open
file": its host path was outside the sandbox. `ExportReceivedFile` now validates
that the path belongs to a completed incoming transfer, opens only a readable
regular non-symlink file, and creates/reuses a nonpersistent read-only document
export for the installed client. GTK then runs its existing native file launcher.
No host-filesystem, system-bus, network or privileged-spawn sandbox permission was
added. Native local-file actions retain their offline behavior.

The Flatpak builder also needed its state directory on the same filesystem as
its temporary build root. This is now explicit, so a checkout on Windows/DrvFS
can build through Linux. The client includes its own AppStream component and
host-service requirement, runtime repository metadata, matching source archive
and checksums. No Flathub publication is claimed.

## Acceptance

`tests/integration/run-flatpak-client.sh` runs the following single journey:

- An unselected host file is unreadable from the sandbox.
- The actual Choose files -> GTK portal -> Add files flow produces the exact
  selected basename and one-file-ready state.
- Share with a link -> Create link starts the real host daemon's HTTP offer.
- The real document export is revoked and the original pathname replaced.
  Download still returns the exact original bytes held by the daemon.
- Stop sharing closes the link; the old download URL is unusable.
- Open file on a seeded completed receipt shows the actual Open With chooser.
  Its selected registered MIME reader executes and reads the exact host bytes.
- Show the destination folder reaches the real OpenDirectory portal and a
  recording FileManager1 service with the correct host URI.
- The exported receipt is readable but not writable inside the sandbox.
  Unreceived, relative and directory paths, and a symlink replacement are rejected.

The native GTK regression passed in 17.67 seconds using
`tests/integration/run-native-gtk.sh`; app/daemon all-target Clippy with warnings
denied also passed. The first native run used standard session service paths:
the old installed daemon auto-activated during simulated owner loss and invalidated
the fixture. The dedicated runner now excludes auto-activation service directories
and isolates preferences, so it tests only the explicitly registered fake owner. Successful installed acceptance takes about eight seconds once the
runtime is installed. Runtime/build setup is not included in that duration.

## Limits and follow-up

The receipt is intentionally seeded in persisted history; it does not prove
physical-device reception. FileManager1 is a recording service, while the MIME
reader is an actual launched process. GNOME/Thunar/Dolphin real manager acceptance
is recorded separately. Export authority is the current regular file at a saved
receipt path, not an immutable inode retained forever after reception.

Custom receive-folder selection must still be checked for host-path resolution
and persistence after portal restart. Booted distribution services, remaining
Shell versions, full screen-reader use and physical radios remain separate work.
The complete goal is still active.

The latest bundle, corresponding source and hashes are in `dist/`; the Ubuntu
companion review package `.15` carries the new host method. Older native host
packages require an upgrade before Flatpak receipt actions can use it.

Primary specifications: [document export and permissions](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Documents.html),
[OpenFile and OpenDirectory](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.OpenURI.html).
