# Completion pass: daemon and storage

Environment: dedicated Ubuntu 24.04 WSL distro `LinuxDrop-Dev`, Rust 1.99.0.
These results cover current completion changes, not the old release artifact.

## Verified

- `cargo test -p linuxdrop-daemon -p linuxdrop-storage --lib --bins`: 8 passed.
  Covers settings validation and live presentation changes, history age boundary,
  remote-name sanitization, peer preferences without invented identity trust,
  path traversal/partial cleanup, collision rename, collision rejection preserving
  existing bytes, and free-space preflight.
- Daemon/storage Clippy with warnings denied passed at the preceding integration
  checkpoint. Protocol adapters are being edited concurrently; this is not a
  claim of a final full-workspace check.
- `dbus-run-session -- python3 tests/integration/daemon_session.py
  /opt/linuxdrop-target/debug/linuxdropd` passed after the latest daemon changes.
  The test uses actual D-Bus and HTTPS on an isolated ephemeral port and isolated
  private config/data directories. It verifies hidden discovery, incoming consent,
  appearance edits during a pending request without interrupting it, exact saved
  bytes, per-request destination and subset, duplicate-decision rejection,
  saved-device blocking/persistence, redacted diagnostics, terminal-state
  stability, private history across restart, clear-history preserving files,
  and clean idle shutdown.

No live-demo restart, physical radio mutation, Android or Apple-device acceptance
was performed by these checks. Native GTK/Shell acceptance is recorded separately.
The existing 0.1.0 release packages must be rebuilt before delivering these changes.
