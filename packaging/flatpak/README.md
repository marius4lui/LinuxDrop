# Optional host-daemon client

This bundle contains only the GTK application. Install the native LinuxDrop package on the host first; its versioned session D-Bus daemon, radio helper and GNOME extension remain host components. Without the daemon the client displays its normal disconnected/error state.

Install Flatpak/flatpak-builder and the GNOME 50 Platform and SDK, then run `sh packaging/build-flatpak.sh` and `flatpak install --user dist/linuxdrop-client.flatpak`. The bundle uses the exact native release executable, so the native release build and matching source archive remain the build inputs; this is not a Flathub source submission.

The sandbox grants access only to the named LinuxDrop session service, display and graphics. It has no system-bus access, host filesystem access, network access or privileged spawn permission. GTK uses the file chooser portal. The client opens selected files inside its namespace and sends Unix descriptors plus validated logical names in bounded D-Bus batches. The host daemon never needs to resolve those sandbox paths. The host daemon remains responsible for validated input, consent and transfer storage. Arbitrary clipboard or portal-less paths are not permission grants.

The GNOME Shell top-panel integration is installed by the native package, not inside the sandbox. The client should be upgraded together with the host package. Native package installation remains the recommended complete configuration.


`tests/integration/run-fd-portal.sh` exercises an actual temporary document-portal
export, its revocation, closed client descriptors and replaced source paths,
followed by exact-byte HTTP downloads from the host daemon in an isolated network
namespace. It also covers 25 files across multiple batches and atomic rejection
of invalid descriptors. The native GTK regression verifies the actual client FD
messages and cleanup after a failed later batch. This proves the portal/descriptor
handoff; installation and file-chooser interaction inside a built Flatpak remain
separate package acceptance requirements.
