# P2P leave and startup recovery - 2026-10-07

`LeaveP2p` previously awaited child/group teardown while holding the helper's
State mutex. It now shares a persistent per-lease group cleanup receipt. Duplicate
requests coalesce; cancelling a waiter does not cancel teardown. The radio stays
reserved and is absent from healthy status while this operation runs. Successful
journal publication clears only the group and restores the direct reservation.
Failed teardown or persistence retains its identity as recovery-only state.

A full retirement queued during group cleanup waits for that receipt outside
State, then reads the final identity before proceeding. Watchdog scans capture a
network-state revision before inventory/address I/O and discard observations if
an intervening radio operation changes it; an old group cannot be judged as the
current group after asynchronous leave/rejoin. The group worker cannot
reattach a lease already selected for full retirement. Pending group formation
is rejected by partial leave; producer cancellation settlement is still a
[separate concrete gap](../completion/NETD_LIFECYCLE.md).

Startup now loads every journaled reservation before accepting requests, starts
background cleanup receipts and services status during recovery. Permission/I/O
errors, invalid JSON and duplicate lease IDs fail startup without treating the
journal as empty. Only a missing journal means a clean initial state. Child
termination/reaping errors are propagated instead of reported as success.
Synchronous durable journal writes still serialize under State; filesystem-stall
latency is not covered by the delayed-network tests.

Verification:

- Helper binary suite: 14 passed; two kernel namespace tests remain explicitly
  ignored by that ordinary command. All-target helper Clippy passed.
- New delayed group scenario verifies responsive Status/RecoveryStatus,
  unauthorized leave rejection, actual authorized LeaveP2p waiting, duplicate
  coalescing, abandoned waiter independence, full-retirement ordering, and use
  of the post-leave group identity. Failed teardown and failed journal cases
  retain the direct lease, its original group and recovery-only status.
- A real debug helper process passed `tests/run-startup-recovery.sh`. The probe
  verifies private mount and network namespaces before mounting private `/run`,
  `/var/lib` and read-only namespace-specific sysfs. It checks old-boot cleanup,
  retained current-boot ownership failure, recovery reads, persistence across
  restart, clean termination/socket removal, and rejection without overwriting
  malformed, duplicate or unreadable journals. Its dummy link is checked to
  survive denied cleanup. The initial probe needed namespace-specific sysfs to
  expose that private link; the corrected final run passed.
- The same startup probe is wired into Linux CI after the release package build.
  This records a local pass and a CI definition, not a remote CI execution.

No installed systemd/polkit lifecycle, real supplicant group teardown or physical
radio acceptance is implied. Acquisition rollback, pre-journal crash handling,
and settled producer cancellation still require implementation/evidence.
