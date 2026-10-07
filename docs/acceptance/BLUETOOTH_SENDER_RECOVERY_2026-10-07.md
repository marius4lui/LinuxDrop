# Quick Share sender advertisements and shared capacity - 2026-10-07

The FastInit sender advertisement now has a supervisor with bounded 2-15 second
backoff. It retries initial controller unavailability, external Release and
bluetoothd replacement. Component readiness is emitted only after registration
acknowledgement, clearing only the sender's earlier error. Transfer workers and
receiver GATT/L2CAP sessions are independent of these advertisement windows.

Sender and receiver advertisements now share a fair Quick Share turn. A queued
sender asks the receiver to yield only when no protected BLE work holds a scan
suppressor. The receiver removes its advertisement and waits for acknowledgement
before the sender registers. The sender advertises for three seconds, then leaves
seven seconds for receiving/scanning. This also makes single-slot adapters usable
without requiring users to disable LinuxDrop's other Quick Share direction.
Other applications' advertisements are never evicted. Existing capacity checks
remain in effect for AirDrop and other processes.

Visibility is checked again after waiting for a turn and before each registration
retry, so a receiver hidden while a sender is active does not reappear afterward.
Cancellation can end a queued turn without registration. Failed cleanup poisons
Quick Share advertisement admission until process restart; neither direction can
claim a new turn with an uncertain previous owner. Registration waits are bounded
at 35 seconds and removal waits at 15 seconds. The vendored BlueR worker keeps
original-owner cleanup alive after timeout. This is deliberately reported as
uncertain cleanup, never successful removal.

The private BlueZ fixture uses one advertising slot and the production sender
and receiver supervisors. It checks initial power-off, protection during a BLE
session, alternating directions without GATT recreation, hiding while queued,
external Release recovery, old-owner cleanup after bluetoothd replacement, cancellation, and removal timeout
preventing another registration. The earlier isolated scanner/advertisement and
receiver-recovery scenarios also run with this scheduler. These are real D-Bus
actors with simulated radios, not evidence of physical phone interoperability.

Outgoing BLE recipient discovery and its one-shot pre-connect scan still need
acknowledged scan ownership, recovery and stronger target correlation. This change
does not claim those separate paths are complete. No browser or live-demo restart.

Validation: the final sender scenario passed in 34.58 seconds, including an
externally released sender registration followed by a new acknowledged one.
The full private-bus runner also passed the existing network advertisement case
(1.17 seconds), scanner/advertisement case (26.36 seconds), and receiver lifecycle
case (26.48 seconds). Quick Share/daemon all-target Clippy passed with warnings
denied. The changed vendor library passes Clippy; its unchanged tx_probe example
still has the recorded dead-code warning. Packaged-daemon evidence is recorded in
`dist/BUILD_REPORT_0.1.0+review.20261007.21.json`.
