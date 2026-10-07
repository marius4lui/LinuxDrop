# Optional host-daemon client

This bundle contains only the GTK application. Install the native LinuxDrop package on the host first; its versioned session D-Bus daemon, radio helper and GNOME extension remain host components. Without the daemon the client displays its normal disconnected/error state.

Install Flatpak/flatpak-builder and the GNOME 50 Platform and SDK, then run `sh packaging/build-flatpak.sh` and `flatpak install --user dist/linuxdrop-client.flatpak`. The bundle uses the native release executable. The build writes a matching `linuxdrop-client_source.tar.gz` and `SHA256SUMS.flatpak` beside it. Runtime-repository metadata lets Flatpak locate GNOME 50; the platform is a separate dependency. This is not a Flathub source submission.

The sandbox grants access only to the named LinuxDrop session service, display and graphics. It has no system-bus access, host filesystem access, network access or privileged spawn permission. GTK uses the file chooser portal. The client opens selected files inside its namespace and sends Unix descriptors plus validated logical names in bounded D-Bus batches. The host daemon never needs to resolve those sandbox paths. The host daemon remains responsible for validated input, consent and transfer storage. Arbitrary clipboard or portal-less paths are not permission grants.

The GNOME Shell top-panel integration is installed by the native package, not inside the sandbox. The client should be upgraded together with the host package. Native package installation remains the recommended complete configuration.


`tests/integration/run-fd-portal.sh` exercises an actual temporary document-portal
export, its revocation, closed client descriptors and replaced source paths,
followed by exact-byte HTTP downloads from the host daemon in an isolated network
namespace. It also covers 25 files across multiple batches and atomic rejection
of invalid descriptors. The native GTK regression verifies the actual client FD
messages and cleanup after a failed later batch. The installed-client test additionally drives the actual GTK chooser inside an
installed bundle, transfers exact bytes through the real host daemon, revokes
the link, opens a completed receipt in a registered host MIME handler, and checks
the real OpenDirectory portal's FileManager1 call. It verifies the sandbox cannot
read arbitrary host paths or write its received-file export.

Received-file actions require the host service's `ExportReceivedFile` method
(Ubuntu review `0.1.0+review.20261007.15` or a matching newer source build). The
service checks completed incoming history and regular-file/no-symlink rules,
then grants only the installed client read access for the current portal session.
GTK retains its normal Open With dialog and folder-reveal behavior. Native GTK
continues to open local receipts without depending on a running sharing service.

Run the installed test as root with a dedicated disposable account that already
has the bundle and GNOME 50 runtime installed:

```sh
sh tests/integration/run-flatpak-client.sh linuxdrop-flatpak-test /absolute/path/linuxdropd
```

Host test dependencies: Xvfb/xauth, xdotool, Python GI/pyatspi, GTK portal,
document portal/FUSE, D-Bus, iproute2, util-linux and Flatpak. The runner creates
private mount/PID/network namespaces, a private session bus and display, and the
standard `/run/user/UID` portal mount. It never uses the running demo's session.
The receive receipt is seeded; this test does not simulate physical radio proof.

Custom receive-folder selection and persistence across portal restarts remain a
separate acceptance item. See [current evidence](../../docs/acceptance/FLATPAK_CLIENT_2026-10-07.md).
