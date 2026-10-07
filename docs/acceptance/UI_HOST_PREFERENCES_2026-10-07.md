# Host Notch preferences from the GTK client

The optional Flatpak previously tried to spawn `gnome-extensions` inside its
sandbox, where the command is absent. Both GTK variants now use the fixed
`OpenNotchPreferences` host action. It calls GNOME's documented
`org.gnome.Shell.Extensions.OpenExtensionPrefs` for LinuxDrop's extension only,
with a ten-second deadline and no arbitrary command, UUID or options input.
The button is disabled during the request, errors are translated, and obsolete
service generations cannot display late results. A failed request permits retry.
The Flatpak gains no filesystem, network or Shell bus permissions.

The installed-client integration exercises the real button, sandbox D-Bus route
and packaged host daemon. A recording GNOME service first rejects the request,
then accepts retry; assertions check the visible failure, restored sensitivity
and exact fixed UUID/empty parent/options. The recording service does not prove
an actual extension window launch. The separate native GNOME preferences smoke
checks the real preferences UI, as recorded in the Shell acceptance reports.

Final package results and hashes are recorded in
`dist/BUILD_REPORT_0.1.0+review.20261007.17.json`; the installed-client log is
`dist/FLATPAK_UX_20261007.log`. The live demo remains unchanged. This closes the
software sandbox route, not physical-device or full-project acceptance.
