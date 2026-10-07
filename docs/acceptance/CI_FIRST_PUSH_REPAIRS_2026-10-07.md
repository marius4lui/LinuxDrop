# First remote CI run: diagnosed failures and repairs

Initial pushed head: `267b200`. Authoritative runs:

- [Native desktop](https://github.com/marius4lui/LinuxDrop/actions/runs/37621043298)
- [Linux](https://github.com/marius4lui/LinuxDrop/actions/runs/37621043337)

The Ubuntu desktop container failed checkout because its minimal runtime lacked
CA certificates. The Fedora desktop container reached GNOME 50 but failed during
Shell initialization because its system bus had no running login1 service.
The workflow now installs CA certificates and python-dbusmock. An explicitly
requested private logind fixture is started on the smoke test's isolated bus;
it models only the test user's local session. Default/live-session execution is
unchanged. Native GNOME 46 and preferences smoke passed locally with this fixture,
German text at 150% and an 800x600 virtual monitor. Fedora confirmation belongs
to the next remote run; a fixture is not a booted systemd/polkit acceptance.

The fuzz workspace did not inherit the root workspace's patched `bluer`, so
AirDrop failed to compile against its acknowledgement methods. Its own manifest
now selects the same local Bluetooth implementation and updates its lockfile.
All three production parser fuzz targets compiled and completed a local bounded
five-second run each without an ASan crash. This is a smoke pass, not a claim of
exhaustive parser coverage.

Dependency policy rejected RUSTSEC-2025-0134 (`rustls-pemfile` unmaintained).
LocalSend now uses the maintained
[PEM parsing API](https://docs.rs/rustls-pki-types/latest/rustls_pki_types/pem/trait.PemObject.html)
and [axum-server 0.8](https://docs.rs/axum-server/0.8.0/axum_server/tls_rustls/struct.RustlsConfig.html).
Its fallible listener construction is handled, and the unused vendored AirDrop
PEM dependency is removed. Root, fuzz and vendored lockfiles are updated. No
advisory is ignored or weakened.

Local evidence: 17 LocalSend tests passed, including pinned HTTPS send/receive,
consent, rejected transfers and draining connections before port reuse.
`cargo deny check licenses sources advisories` passed; LocalSend and daemon
all-target Clippy passed. The network receipt tests are recorded separately.
Remote green status is not inferred from these local checks, and existing DEB
revision 31 is not described as containing these subsequently changed binaries.
