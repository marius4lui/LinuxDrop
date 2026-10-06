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

## 2026-10-07: confirmed backend restart

- Replaced the fixed 500 ms restart delay with retained backend completion receipts.
  A timeout preserves cleanup and blocks replacement; repeated shutdown observes
  the same receipt. Backend actor panic cannot count as successful teardown.
- LocalSend waits for accepted TLS sockets as well as retired network listeners,
  discovery, maintenance and outgoing jobs. AirDrop tracks connection-cancellation
  guards and awaits mDNS shutdown; Quick Share no longer ignores stop timeouts.
- Workspace tests passed: 51 executed, 3 environment-dependent tests skipped.
  The skipped native GTK regression then passed separately in its private
  Xvfb/session bus. Full-workspace/all-target Clippy passed with warnings denied.
- The actual D-Bus/HTTPS integration passed with a deliberately stalled accepted
  TLS socket. It holds restart admission closed until the client disconnects,
  retains the selected file descriptor, rejects duplicate restart/settings/send
  admissions, and then resumes normal validation on the replacement listener.
- The actual document-portal/Unix-FD integration passed again, including revoked
  portal exports, original bytes after path replacement, 25-file batches and
  invalid-descriptor rejection.

These checks do not cover BlueZ unregister acknowledgement, helper-loss recovery,
reverse-offer drain or installed-package acceptance. No live demo was restarted.
