# Bluetooth signal resource bounds - 2026-10-07

The vendored bluez-async subscription now owns one shared FIFO for all of its
D-Bus signal matches. It retains at most 256 messages and 2 MiB of serialized
payloads. The budget is released as messages are consumed. This is a retained
payload bound, not a total process RSS claim: libdbus, allocator bookkeeping and
one message being delivered/marshalled use additional memory.

Exceeding either limit closes the entire subscription, discards its stale queue
and wakes its consumer. Further callbacks cannot enqueue anything. This avoids
silently losing one event while keeping an apparently healthy partial state.
An overflow in PropertiesChanged also ends the companion InterfacesAdded stream;
it cannot leave select_all waiting forever for the other match. Dropping the
subscription removes all match rules through the existing bounded cleanup and
retains the connection worker until those removals have been attempted.

FastInit scanning now treats an ended event stream as a failure. Its supervisor
reports the component error, acknowledges scan removal and rebuilds with backoff.
Recipient scans already treat an ended stream as an error and retain their owned
cleanup. Neither path starts a replacement scan before removal acknowledgement.

Validation includes queue count overflow, shared byte limits, single oversized
message rejection, FIFO order and 600 drain/refill cycles. A private D-Bus test
sends three 1024-event floods through the real BlueZ subscription, proves each
entire subscription terminates, and verifies a new subscription receives a normal
state change. Its method-reply barrier follows the emitted signals on the same
service connection; it does not rely on a timing-only guess that the flood arrived.
The same fixture injects an oversized signal into production FastInit scanning,
holds StopDiscovery, verifies no replacement while removal is pending, then
observes automatic recovery. Foreground recipient scanning fails and cleans up
under the same overload. No system Bluetooth bus or physical radio is used.

The focused D-Bus test passed in 2.94 seconds. Three queue tests and both changed
vendor libraries plus Quick Share/daemon all-target Clippy pass with warnings
denied. The standard private Bluetooth runner now includes both new suites using
a committed standalone vendor test lockfile. The final full runner passed all six private-bus scenarios: advertisement
(1.17 s), listener (37.37 s), receiver (26.48 s), sender (34.58 s), recipient
(16.13 s), and signal bounds (2.95 s). Packaged-daemon results belong in
`dist/BUILD_REPORT_0.1.0+review.20261007.23.json`.

This closes the previously identified unbounded bluez-async signal queue. It does
not prove long-term RSS behaviour of all Bluetooth roles, BlueZ's own device cache,
other independent signal consumers, cross-protocol airtime or physical phone
interoperability. Those remain separately tracked. No live-demo restart.
