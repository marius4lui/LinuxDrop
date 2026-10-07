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
  Unidentified late GroupStarted events and ambiguous Cancel results now retain
  a durable formation intent and radio reservation, including across helper
  restart. Automatic same-boot reconciliation remains open; see
  [uncertain formation evidence](../acceptance/P2P_UNCERTAIN_FORMATION_2026-10-07.md).
  Known unmarked groups retain strict ownership checks during recovery.
  Known-group cleanup now waits for live supplicant
  inventory removal and, in netd, kernel interface disappearance; see
  [removal observation evidence](../acceptance/P2P_REMOVAL_OBSERVATION_2026-10-07.md).
- Acquire/AcquireAwdl now use persistent producers and cleanup receipts outside
  State; Reserve inventory no longer holds State either. See
  [acquisition evidence](../acceptance/NETD_ACQUISITION_2026-10-07.md).
  SetChannel now uses the same producer ordering, validates again after inventory
  and revokes uncertain mutations. See
  [channel retirement evidence](../acceptance/NETD_CHANNEL_RETIREMENT_2026-10-07.md).
  Creation before verifiable owner-marker publication still needs crash-recovery
  handling; durable intent alone cannot prove ownership.
- Startup now listens while journaled leases recover through receipts. A real
  helper process in private mount/network namespaces passes old-boot cleanup,
  retained ownership failure across restart, normal shutdown, and malformed or
  unreadable journal rejection. A separate booted Ubuntu/systemd installation
  now passes real unit identity/capabilities, restart/stop/start, malformed journal
  recovery and inactive-user denial; see
  [installed service evidence](../acceptance/BOOTED_UBUNTU_SERVICE_2026-10-07.md).
  Real terminal-agent Polkit authorization now passes active/inactive/seatless,
  no-agent and wrong/correct password cases after fixing the mechanism action-owner
  annotation; see [authorization evidence](../acceptance/POLKIT_AUTHORIZATION_2026-10-07.md).
  Actual user-service invocation and remote sessions (including concurrent local
  login of the same UID) pass again on installed `.36`. Booted Ubuntu upgrades,
  ordinary package removal and reinstallation now pass, preserving the service
  UID and journal. Native GNOME 46 graphical authorization/cancellation now also
  passes in English/German; see [rendered authorization evidence](../acceptance/GNOME_POLKIT_2026-10-07.md).
  Other distro package lifecycles, other Shell versions and the remote CI
  user-service startup result remain open.
  The CI probe now derives its test user's home from passwd and runtime directory
  from logind, verifies runtime/bus ownership, and temporarily supplies those XDG
  values to that user's systemd manager. An installed `.40` daemon passed the
  focused identity/confinement/D-Bus file-handoff check with deliberately foreign
  runtime/config values inherited by its PAM session. The manager environment is
  restored afterwards. This changes only the disposable test setup; the remote
  full CI result remains to be confirmed.
- Re-run relevant isolated kernel network and installed-package lifecycle checks
  after the completed helper change. Unit fixtures are not physical radio or
  installed daemon acceptance.
