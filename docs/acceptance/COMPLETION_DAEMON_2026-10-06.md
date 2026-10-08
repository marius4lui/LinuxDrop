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

## 2026-10-07: link lifetime and idle shutdown

- ReverseOffer now confirms HTTP connection and blocking-file-reader cleanup on
  stop/expiry; same-port replacement no longer uses a fixed 100 ms sleep. Cleanup
  timeout/failure stays observable and retryable through ShutdownReceipt.
- Admission quiescing preserves already-admitted payloads for StopWhenIdle. New
  requests are rejected. Explicit stop still cancels active payloads. Link
  replacement refuses active streams atomically and preserves the prepared draft.
- Live links block network settings/restarts; snapshot `download_link_active`
  exposes sharing state without publishing PINs/URLs. Failed cleanup remains
  visible until a successful receipt.
- Core and LocalSend tests: 21 passed, including blocked payload cancellation,
  partial HTTP requests, expiry, port reuse and quiescing with exact saved bytes.
  Daemon unit tests: 5 passed. Workspace/all-target Clippy passed.
- The extended private-network/document-portal integration passed with actual
  D-Bus/HTTP: active-link restart protection, confirmed revocation, immediate
  same-port replacement, refused replacement during a download, HTTP 410 after
  idle quiescing and a complete 512 KiB payload before daemon exit.
- The ordinary daemon HTTPS/consent integration passed. Native German GTK
  regression passed, including failed-stop visibility and retry. The 480x600
  active-link card was rendered and inspected (`ui/completion/active-download-link.png`).

No running demo or installed package was changed. BlueZ unregister and helper-loss
recovery are still separate open lifecycle work.

## 2026-10-07: acknowledged Bluetooth advertising cleanup

The workspace now pins BlueR 0.17.4 sources with a focused lifecycle extension.
AirDrop and Quick Share await UnregisterAdvertisement; Quick Share cycles stop
using a guessed 1.5-second grace period. Registration cancellation is owned by a
worker until the D-Bus reply and cleanup. Each advertisement uses an isolated
D-Bus owner without disconnecting the controller/GATT session. The source and
license are retained and packaging installs the BlueR license.

The private-bus BlueZ test passed after reproducing and fixing an additional
session teardown problem: event/method dispatch tasks retained connection clones
after the I/O driver had been stopped. The test now verifies the bus owner
actually disappears, as well as explicit/automatic powered-controller choice,
Apple manufacturer data, Quick Share service/discoverability/interval properties,
full capacity, a held unregister reply, transient-error retry, late registration
cleanup after caller cancellation and handle Drop cleanup. No real BlueZ service
or physical radio is contacted. CI includes the isolated test.

Full-workspace tests passed (53 executed, 4 environment-dependent tests skipped);
the new private BlueZ lifecycle test passed separately. Workspace/all-target
Clippy with warnings denied passed. The live demo was not restarted.

Physical advertising, external Release/power-loss recovery and the complete
scanner/GATT/L2CAP controller matrix remain open; this test only proves the
specific software lifecycle above.
