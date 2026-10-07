# Quick Share recipient scans - 2026-10-07

Recipient discovery and pre-connect scans now use the selected controller and a
shared scan turn with the FastInit listener. A worker retains ownership through
StartDiscovery and StopDiscovery replies even when the calling transfer is
aborted. Cleanup uncertainty blocks replacement; one original-owner cleanup task
can restore admission after confirmed removal. A scanner waiting for the turn
continues monitoring controller loss. Server shutdown and listener errors drain
transfer tasks and await outstanding recipient scan cleanup.

Recipient selection correlates the advertised endpoint ID instead of a display
name or rotating Bluetooth address. Equal names remain separate devices. This
identifier is discovery correlation, not authentication; the encrypted handshake,
SAS comparison and consent remain required. Cached btleplug discovery entries are
drained before scanning. Only current events refresh a peer; unseen peers expire
after 30 seconds, and a controller failure removes the current generation.
Background scanning retries ordinary failures with bounded backoff and reports
its own component readiness. Scan results are limited to 512 endpoints.

Cancelling a pending BLE connection cancels only that transfer and emits the
Cancelled state without subsequently replacing it with Disconnected. Backend
shutdown waits for the scan removal receipt. The BlueZ D-Bus IO task now belongs
to its sessions and signal streams, including bounded match-removal cleanup;
recreating scans no longer leaves persistent D-Bus client connections behind.

The private BlueZ fixture exercises actual production workers and D-Bus calls
with simulated adapters: equal names with different endpoint IDs, stale cache,
fresh advertisements, held Start/Stop replies, caller abort, offline recovery,
peer withdrawal, unrelated versus matching cancellation, backend shutdown,
original-owner cleanup after daemon replacement, failed cleanup preventing new
scans and later recovery. Repeated scans check stable D-Bus client count.

The complete private-bus suite passed: network advertisement (1.15 s), scanner
and advertisement lifecycle (31.36 s), receiver lifecycle (26.49 s), sender
lifecycle (34.57 s), and recipient scan (16.13 s). Quick Share/daemon all-target
Clippy and both changed vendor libraries pass with warnings denied. The final
listener-error cleanup adjustment is covered by recompilation and a repeat of
the recipient shutdown scenario (16.12 s). Four Quick Share library tests passed,
including encrypted exact-byte transfer with both consents, alongside three
Bluetooth task ownership regressions. Package evidence belongs in
`dist/BUILD_REPORT_0.1.0+review.20261007.22.json`.

Remaining acceptance: physical phone/radio interoperability, long-running event
flood/resource bounds and shared airtime with other protocols. The underlying
BlueZ signal queue is still unbounded; the result/backlog limits above do not
claim to solve that separate concern. No live-demo restart or browser use.

Follow-up: the bluez-async signal queue limitation above is addressed by
[bounded subscriptions](BLUETOOTH_SIGNAL_BOUNDS_2026-10-07.md). Broader long-running
resource and physical acceptance remain open.
