# Network helper lifecycle completion

Current full-retirement receipts are recorded in
[acceptance](../acceptance/NETD_CLEANUP_RECEIPTS_2026-10-07.md).
Do not treat that bounded pass as full helper completion.

Remaining code/evidence requirements:

- `LeaveP2p` now uses a persistent receipt outside State and full retirement
  waits for it, preserving the direct reservation until journal publication.
  See [partial cleanup and startup evidence](../acceptance/NETD_GROUP_RECOVERY_2026-10-07.md).
- The network group's Drop cleanup now provides a settlement receipt, and
  client/host creation errors after guard creation await it. See
  [network acknowledgement evidence](../acceptance/P2P_GROUP_SETTLEMENT_2026-10-07.md).
  `join_p2p` failure cleanup and producer cancellation still need to integrate
  this receipt through full helper retirement. A full release must not admit radio reuse while an old group producer
  or cleanup can still act. The current cleanup-running check prevents a known
  already-started full cleanup from being duplicated, but is not a complete
  race-free group-operation protocol.
- Failed Acquire/AcquireAwdl rollback still waits under State, and creation before
  durable journal publication needs a crash-recovery audit/fix.
- Startup now listens while journaled leases recover through receipts. A real
  helper process in private mount/network namespaces passes old-boot cleanup,
  retained ownership failure across restart, normal shutdown, and malformed or
  unreadable journal rejection. Booted systemd/polkit lifecycle remains separate.
- Re-run relevant isolated kernel network and installed-package lifecycle checks
  after the completed helper change. Unit fixtures are not physical radio or
  installed daemon acceptance.
