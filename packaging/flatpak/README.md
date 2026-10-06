# Optional host-daemon client

This bundle contains only the GTK application. Install the native LinuxDrop package on the host first; its versioned session D-Bus daemon, radio helper and GNOME extension remain host components. Without the daemon the client displays its normal disconnected/error state.

Install Flatpak/flatpak-builder and the GNOME 50 Platform and SDK, then run `sh packaging/build-flatpak.sh` and `flatpak install --user dist/linuxdrop-client.flatpak`. The bundle uses the exact native release executable, so the native release build and matching source archive remain the build inputs; this is not a Flathub source submission.

The sandbox grants access only to the named LinuxDrop session service, display and graphics. It has no system-bus access, host filesystem access, network access or privileged spawn permission. GTK uses the file chooser portal; selected document paths are passed to the host daemon, which can read the host document portal. The host daemon remains responsible for validated input, consent and transfer storage. Arbitrary clipboard or portal-less paths are not permission grants.

The GNOME Shell top-panel integration is installed by the native package, not inside the sandbox. The client should be upgraded together with the host package. Native package installation remains the recommended complete configuration.
