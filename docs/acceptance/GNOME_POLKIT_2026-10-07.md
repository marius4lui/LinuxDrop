# Installed GNOME authorization - 2026-10-07

Package `.37` was tested in the separate booted Ubuntu 24.04 acceptance system
with GNOME Shell 46, real logind/PAM, the installed user/system services and the
unchanged installed Polkit action. No live demo or existing desktop was replaced.
The dedicated test user's headless Wayland desktop uses software rendering.

`netd_polkit.py --graphical` extends the existing terminal/session matrix with
two real desktop flows. Both pass in English and German:

- The installed user daemon sends its hardware diagnostic through the helper.
  GNOME's own registered Polkit agent opens the native authorization dialog.
  The action/message belong to LinuxDrop; password visibility is disabled and
  keyboard focus is already in the password field. Cancelling the dialog denies
  the operation.
- A fresh graphical session displays the same request. Entering the synthetic
  administrator password through the native password-entry activation reaches
  radio selection (`radio not found` for the intentionally nonexistent radio).
  No actual radio is configured and no sharing backend is enabled in the test.

The test-only extension operates the installed GNOME dialog; it does not replace
the dialog, authenticate by a test rule, bypass PAM, or mock the service response.
Capture occurs only after desktop startup and modal opening have settled, before
any password is entered. The initial early capture exposed a startup/Overview
transition; waiting for actual rendered state corrected that test timing issue.
The final 1280x800 German cancel/confirm captures were visually inspected: clear
German message, complete controls, focused empty password field and no clipping.

The generated credential is received with echo disabled in the private PTY and
passed to the isolated Shell only in memory/environment. No credentials appear
in arguments, screenshots or reports. The account is locked in finally; the
Shell, daemon and temporary test extension are stopped/removed on completion.

Local artifacts (ignored build outputs):

- `dist/GNOME_POLKIT_0.1.0+review.20261007.37.json`
- `dist/polkit-gnome-de/gui-cancel.png`
- `dist/polkit-gnome-de/gui-authenticate.png`

A dedicated CI job downloads the package produced by the same workflow run,
checks its archive hashes, installs real GNOME/PAM dependencies and repeats the
German flows. Its first remote result remains pending. The upstream build run
`37651592358` no longer reports systemd's capability setup failure, but the daemon
itself exits with status 1 during startup on that runner. Narrow startup logs and
an allowlist of desktop environment fields are now included for diagnosis; this
remote difference is not counted as passed by the local graphical result.

Physical adapter mutation and graphical behavior on other supported GNOME
versions remain separate acceptance. This report does not claim full project
completion.
