# Actual helper and daemon AWDL retirement

The production netd watchdog now revokes leases when inventory reports rfkill,
in addition to interface/child/owner loss, regulatory restrictions and competing
radio use. Previously a blocked radio could keep a live process and interface
and continue to appear leased/ready. Acquisition already excluded blocked radios;
this change covers a block applied during an active lease.

## Evidence

The release netd executable passes nine scenarios through its real Unix IPC,
watchdog loop, durable journal, child reaping and cleanup workers:

- Child exit, including a held interface-deletion acknowledgment: status revokes
  immediately, the journal keeps the reservation, and another acquisition fails
  until cleanup finishes.
- TAP loss, monitor loss, radio disappearance and rfkill.
- A newly restricted channel and a competing active interface.
- Changed interface ownership: the helper reaps its own child/TAP but preserves
  the unverified monitor and reservation; restoring its marker then retrying
  recovery cleans only the owned resource.
- Client socket EOF.

Every case checks child reaping and nonpersistent TAP disappearance. Owned
monitors and journal reservations disappear only after successful cleanup.
The competing foreign interface survives. A new acquisition succeeds for the
next scenario rather than inheriting the preceding lease.

The same fixture then starts the **actual release linuxdropd** against that
helper. AirDrop reaches ready over its public D-Bus snapshot and opens its real
IPv6 TCP listener on port 8771. A channel restriction is injected; the watchdog
revokes the lease and the daemon publishes error and retires its backend. The
journal and TAP are cleared. `RestartBackends` then creates a distinct lease and
reopens the real listener. Stopping the helper in that second generation again
produces error and complete cleanup. All eleven scenarios passed locally.

## Boundaries and reproduction

```sh
sudo sh crates/linuxdrop-netd/tests/run-awdl-watchdog.sh /path/linuxdrop-netd /path/linuxdropd
```

The wrapper creates private mount and network namespaces and a private D-Bus.
The fixture asserts both namespaces differ from PID 1 before any mount/write.
`/run`, `/var/lib`, radio sysfs metadata and the helper executable directory are
private overmounts. A radio fixture replaces iw and Filin only inside them;
actual kernel dummy links and a real nonpersistent TAP are used. The fixture
provides synthetic radio capabilities and logind; pkcheck is a private success
stub because real authorization already has separate installed-system evidence.
It does not modify installed host binaries, the live demo or host network links.

This proves software integration of the real helper, user daemon, AirDrop
listener and published backend state. It does **not** prove AWDL radio traffic,
Apple interoperability, physical rfkill/unplug behavior or authorization. The
real Filin failed-channel startup path is separately covered by
[managed-link acceptance](AWDL_MANAGED_LINK_2026-10-07.md); physical channel
hopping/injection and peer acceptance remain open. CI runs this fixture after
installing the package; its latest remote result is pending.
