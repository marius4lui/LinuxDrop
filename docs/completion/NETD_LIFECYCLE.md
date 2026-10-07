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
  Full helper retirement now waits for its persistent group/address producer;
  socket abandonment requests cancellation and DHCP is reaped before settlement.
  See [producer retirement evidence](../acceptance/NETD_PRODUCER_SETTLEMENT_2026-10-07.md).
  Unidentified late GroupStarted events and ambiguous Cancel results still need
  explicit recovery semantics; known unmarked groups must retain strict ownership
  checks during recovery.
- Acquire/AcquireAwdl now use persistent producers and cleanup receipts outside
  State; Reserve inventory no longer holds State either. See
  [acquisition evidence](../acceptance/NETD_ACQUISITION_2026-10-07.md).
  SetChannel still awaits an external command under State and needs coordinated
  producer retirement. Creation before verifiable owner-marker publication still
  needs crash-recovery handling; durable intent alone cannot prove ownership.
- Startup now listens while journaled leases recover through receipts. A real
  helper process in private mount/network namespaces passes old-boot cleanup,
  retained ownership failure across restart, normal shutdown, and malformed or
  unreadable journal rejection. Booted systemd/polkit lifecycle remains separate.
- Re-run relevant isolated kernel network and installed-package lifecycle checks
  after the completed helper change. Unit fixtures are not physical radio or
  installed daemon acceptance.
