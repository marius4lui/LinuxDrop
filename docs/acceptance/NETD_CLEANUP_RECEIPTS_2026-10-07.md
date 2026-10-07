# Network-helper retirement receipts - 2026-10-07

Full lease retirement previously awaited child exit and network restoration while
holding the global helper State mutex. Status and recovery reads, cancellation,
and unrelated clients could consequently wait behind a slow restoration.

Full Release, client EOF, RetryRecovery, watchdog retirement and shutdown now
share a persistent per-lease cleanup worker and completion receipt. Starting the
worker detaches the lease from healthy status but retains its reservation and
journal identity. Slow process/network work runs outside State. Duplicate full
retirements observe the existing receipt; dropping a waiter does not cancel the
worker. The supervisor turns a stopped/panicked I/O task into a failed receipt.
The journal is updated before successful publication; an update failure restores
the in-memory reservation for explicit recovery. Error history is capped at 128
entries, while RecoveryStatus continues returning its newest 32 entries.

The watchdog also performs its potentially slow address lookup outside State
and starts retirement without waiting for the entire radio cleanup. Read-only
RecoveryStatus distinguishes in-progress cleanup from an orphaned failed lease.
Existing connection ownership checks still gate Release and partial P2P leave.
Durable journal writes remain synchronously serialized under State; the response
time assertions cover delayed process/network futures, not a stalled filesystem.
Client EOF, recovery and shutdown start retirement on all selected leases before
waiting for the first slow adapter. Shutdown awaits the same outstanding
receipts before final journal persistence.

Verified evidence:

- Final helper binary suite: 11 passed, 2 explicitly ignored namespace tests.
- Final targeted receipt tests: 2 passed. Held completion proves bounded Status
  and RecoveryStatus response while retiring, unavailable healthy status, retained
  allocation, duplicate coalescing, unauthorized Release rejection, partial leave
  exclusion, cancelled waiter independence, failed Release retaining ownership,
  failed journal publication, and successful retry.
- A real owned sleep child is killed and reaped before the receipt succeeds. Its
  synthetic previous-boot lease prevents network mutation, and test-injected
  journal functions never touch the production journal. The Release waiter is
  explicitly polled into the existing injected operation before it completes;
  it cannot accidentally start production cleanup during the test.
- Helper all-target Clippy and formatting/diff checks passed.

This closes the full-retirement global-lock wait, not every helper lifecycle
issue. Partial P2P group cleanup, producer cancellation acknowledgement, failed
acquisition rollback and the pre-journal crash window still require the separate
[lifecycle follow-up](../completion/NETD_LIFECYCLE.md). These are software work,
not physical-hardware exemptions. No physical radio recovery, live service
restart, browser use or live-demo modification is claimed by these tests.
