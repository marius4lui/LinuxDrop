# Persistent monitor/AWDL acquisition - 2026-10-07

Monitor creation previously held the helper State mutex through external
commands, hardware rescans, AWDL startup and failure rollback. It also recorded
socket ownership only after preparation finished. Consequently status could wait
behind network timeouts, and a dropped preparation had no settled producer.

Acquisition now has a dedicated module. It scans hardware before locking State,
validates current reservations under State and persists the lease intent before
any mutation. Socket ownership and a persistent radio producer are registered
before yielding. EOF during Acquire, AcquireAwdl or Diagnose requests cancellation;
shutdown/recovery waits for that producer before restoring its lease. A late AWDL
child is handed to State even if cancellation won. Only successful preparation
attaches the lease; failed preparation waits for the existing full cleanup receipt.
Failed cleanup keeps the identity reserved. A diagnostic uses the pre-operation
ownership set when deciding whether a failed acquisition was actually restored.

Direct-Wi-Fi Reserve inventory also runs outside State. The final reservation
check and journal update remain serialized, so concurrent requests cannot both
reserve a radio based on their earlier scans. Durable filesystem publication
still happens under State; this does not claim bounded filesystem-stall latency.

Network command timeouts now explicitly terminate and reap the child before
returning. stderr is drained concurrently, with at most 64 KiB retained for the
error message. Child diagnostics cannot keep the caller waiting indefinitely.

Verification:

- Helper binary suite: 18 passed, two isolated network tests explicitly ignored.
- Delayed acquisition fixtures verify responsive Status/RecoveryStatus, ordering
  against queued retirement, an actual late sleep process reaped before receipt,
  no attachment after cancellation, and retained ownership after rollback failure.
- A real sleeping subprocess is absent from /proc before command timeout returns.
- Both kernel namespace tests pass separately: IPv6/client DHCP cancellation and
  group-owner DHCP without router/DNS options.
- All-target helper Clippy passes with warnings denied.

This proves the coordination and subprocess paths, not physical monitor creation
or AWDL interoperability. The creation-to-owner-marker crash window and recovery
of known unmarked interfaces remain open. SetChannel still performs its bounded
external command under State and needs the same producer/retirement coordination.
Booted systemd/polkit acceptance remains separate from namespace process tests.
