# Installed service on booted Ubuntu/systemd - 2026-10-07

A separate WSL2 distribution, LinuxDrop-Acceptance, was imported from
[Canonical Ubuntu Base 24.04.5 amd64](https://cdimage.ubuntu.com/ubuntu-base/releases/24.04/release/).
The SHA256SUMS signature verified against Ubuntu's installed archive keyring
(Ubuntu CD Image Automatic Signing Key 2012); the archive hash was
`e77b6f10c2590cef872b33ee9f635a0e3fd1f57fb074c0e52b5c7f56147a0c86`.
Its disk and bootstrap inputs are ignored local artifacts under
`.dev/systemd-acceptance/`. Only this new distribution was restarted. The existing
LinuxDrop-Dev demo was not restarted or replaced.

The acceptance OS boots systemd as PID 1, with real D-Bus, logind and Polkit
packages. LinuxDrop DEB `0.1.0+review.20261007.34` installed successfully with
`--no-install-recommends`; no NetworkManager or Bluetooth service was added to
manage shared WSL networking. Its normal postinst created the helper account and
enabled/started the real systemd service. This is a minimal booted Ubuntu service
environment, not a complete GNOME desktop or physical-radio acceptance.

`tests/integration/netd_systemd.py` requires root, systemd PID 1, an explicit
disposable-system opt-in and a dedicated /etc marker. It refuses to mutate a
nonempty journal. The installed-service run passed:

- Actual service process UID is linuxdrop-netd, not root; effective and bounding
  capabilities are exactly NET_BIND_SERVICE, NET_ADMIN and NET_RAW; NoNewPrivs=1.
- Journal owner/mode 0600 and state directory 0700, socket 0666.
- An actual unprivileged process can query status, but a radio reservation from
  an account without a local logind session is denied before mutation.
- Restart changes the process; orderly stop removes the socket and retains the
  journal; start serves status again.
- A deliberately malformed empty-test journal fails startup without being
  overwritten. A finally block restores the original journal; the real service
  then starts successfully again.

This closes the previously unexecuted booted systemd/installed-service baseline
for this Ubuntu configuration. Interactive Polkit authorization, the full active/
inactive/remote session matrix, package upgrade/remove under this booted OS,
other distributions, and physical radio lifecycle remain open. Earlier namespace
probes and GNOME smoke results are separate evidence, not substitutes for those
requirements.

The package hash is
`f6e0588b17bd8fe0c1951b771e55bd025c492c592734885f6154e80799dae0e4`.
Its production source corresponds to commit 330e3bd; this subsequent acceptance
script/report does not change its binaries.

The Linux CI package smoke step now runs the same installed-service probe before
removing the DEB and publishes its JSON report with the package artifact. This is
local acceptance plus a CI definition; the new remote execution remains pending.

## Follow-up: installed package .36

Package `0.1.0+review.20261007.36`, production commit `a00eab7`, upgrades `.35`
successfully in the same booted Ubuntu environment. Its installed systemd probe
and all eight real PAM/Polkit scenarios pass; all four installed executable
hashes match the release build. Authorization scope and the separate remote CI
issue are described in [the Polkit report](POLKIT_AUTHORIZATION_2026-10-07.md).

The new root/marker-gated `tests/integration/package_lifecycle.py` then performs
actual package removal and reinstallation. Removal stops the helper and removes
its socket, service file and executables. The recovery journal and dedicated UID
remain intact. Reinstallation restores the same version, after which the actual
systemd identity, restart and malformed-journal recovery probes pass again.
The probe refuses a nonempty lease journal or running user daemon and reinstalls
in finally even if a removal assertion fails. CI now publishes its JSON report.

DEB SHA-256:
`2fad53bf1eadcb9509196c3a3a2a56f9ec9b14657be4c94846bf6771fa4f1b13`.
This closes ordinary Ubuntu removal/reinstallation without active transfers.
Physical-radio removal during transfers and other distro package-manager
lifecycles remain separate acceptance.
