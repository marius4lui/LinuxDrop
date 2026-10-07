# Network helper lifecycle completion

Current full-retirement receipts are recorded in
[acceptance](../acceptance/NETD_CLEANUP_RECEIPTS_2026-10-07.md).
Do not treat that bounded pass as full helper completion.

Remaining code/evidence requirements:

- `LeaveP2p` still holds State during child/group teardown. Move partial cleanup
  onto an acknowledged per-lease operation, preserving the underlying direct
  reservation. Full retirement must serialize with or supersede this operation.
- `join_p2p` failure cleanup and producer cancellation need an explicit settled
  receipt. A full release must not admit radio reuse while an old group producer
  or cleanup can still act. The current cleanup-running check prevents a known
  already-started full cleanup from being duplicated, but is not a complete
  race-free group-operation protocol.
- Failed Acquire/AcquireAwdl rollback still waits under State, and creation before
  durable journal publication needs a crash-recovery audit/fix.
- Startup restoration occurs before listening. Verify bounded recovery/startup
  behavior and service stop/restart with retained journal errors in an isolated
  booted systemd/polkit environment.
- Re-run relevant isolated kernel network and installed-package lifecycle checks
  after the completed helper change. Unit fixtures are not physical radio or
  installed daemon acceptance.
