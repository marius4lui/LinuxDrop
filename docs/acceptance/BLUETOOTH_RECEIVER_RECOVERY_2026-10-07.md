# Quick Share receiver recovery - 2026-10-07

The receiver now supervises GATT, L2CAP and its advertisement as one radio
generation. All use the same selected adapter and the newly bound L2CAP PSM;
the advertised endpoint cannot retain an old listener port after recovery.
Controller power/owner loss or an ended worker cancels the generation, confirms
cleanup, then retries with bounded 2-15 second backoff. Initial unavailability
keeps the requested adapter selected rather than falling back to another one.
When L2CAP is unavailable, GATT remains the documented receive fallback.

Inbound sessions have a bounded backend-owned task group. GATT notification
bridges and both L2CAP wire dialects remain generation-owned; already migrated
payloads are not cancelled with their old radio bridge. Backend shutdown closes
both groups. This ownership was checked with actual duplex bytes after generation
shutdown; that fixture does not itself perform a Bluetooth-to-Wi-Fi protocol
upgrade or certify physical L2CAP behavior.

Readiness is reported per component only after registration acknowledgement
(or deliberately disabled advertising while invisible). Cancelled registration
cannot emit a late GATT ready event. Controller failure is reported before
cleanup, so the desktop does not retain a ready status while recovery waits.
Unconfirmed cleanup is terminal for receiver re-registration. The supervisor
retains migrated sessions until backend shutdown and surfaces the cleanup error.
GATT has a 35-second registration deadline and 15-second cleanup deadline;
BlueR retains its original-owner cleanup worker after either wait expires.

The private BlueZ fixture exercises the production supervisor through initial
power-off, restoration, held cleanup across the retry interval, same-controller
recovery, replacement bluetoothd with the old owner still reachable, cancellation
during registration, and an expired cleanup deadline. The latter initially
exposed an unbounded wait and now passes without duplicate registration.
The GATT mock also accepts repeated removal after a delayed reply, matching the
real cleanup worker's retry behavior. Final receiver scenario: 1/1, 26.48 seconds.
The existing scanner/advertisement suite passed separately (20.36 seconds), as
did the network advertisement lifecycle case (1.17 seconds).

This does not complete sender advertisement or outgoing recipient-discovery
recovery. Real Apple/Android interoperability, controller unplug, and physical
L2CAP rebinding remain separate acceptance. No browser or live-demo restart was
used. Package and final compiler/test receipts accompany the review artifact.

Final focused validation: four Quick Share library tests pass, including actual
UKEY2 consent and exact received file bytes; all three bounded task-lifetime
regressions pass. Network, Quick Share, daemon and vendored protocol Clippy
complete successfully; the unchanged vendor tx_probe example retains its existing
dead-code warning. Package verification is recorded in
`dist/BUILD_REPORT_0.1.0+review.20261007.20.json`.
