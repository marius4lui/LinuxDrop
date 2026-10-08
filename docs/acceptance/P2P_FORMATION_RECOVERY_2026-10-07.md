# Pending Wi-Fi Direct formation recovery, 2026-10-07

Previously every current-boot `p2p_pending` record required a reboot because
the journal contained no provenance for a safe cancellation. Netd now records
the original supplicant unique owner, authenticated bus GUID, parent object,
interface inventory and kernel interface indices before either hosting or
joining a group. Both protocol entry points recheck this prepared identity
before submitting Find/Connect/GroupAdd. A failed journal write prevents submission.

Startup, explicit retry and a bounded periodic retry can now reconcile a pending
formation on the original service instance. Recovery requires unchanged kernel
interfaces, the original parent/owner/bus and no group or unidentified interface.
It sends Cancel to that unique owner, requires its successful reply, then repeats
the uncached supplicant inventory and kernel checks before releasing the radio.
It never uses Disconnect to remove an unidentified group. This follows the
[supplicant API distinction](https://w1.fi/wpa_supplicant/devel/dbus.html) between
cancelling formation and disconnecting a group.

The periodic pass considers only unattached pending records with provenance and
no active producer/cleanup, every 30 seconds after its initial pass. Existing
persistent cleanup receipts still serialize retirement and journal publication.
Old journal records deserialize without the added optional field and retain
their conservative behavior. A proven removed wiphy may retire its recorded
operation without touching a replacement adapter.

Verification:

- Seven private-bus supplicant scenarios pass, including current-owner recovery,
  failed Cancel, foreign bus/owner, existing and late unidentified groups,
  changed preparation inventory and no submission after preparation mismatch.
- 22 ordinary helper tests and 14 daemon tests pass. Two existing kernel tests
  and the daemon's separate AWDL test are not part of that ordinary command.
- Kernel-provenance checks detect recreated interface indices, additional links
  and unreadable indices rather than treating errors as an empty inventory.
- A real netd executable in private mount/network namespaces passes journal
  reload, process restart, automatic recovery after a refused cancellation,
  and refusal of foreign owner/group/kernel state. Its sysfs and supplicant are
  controlled fixtures; this is an actual helper process, not physical radio
  acceptance. CI runs this fixture against the release helper.
- Targeted network/helper/daemon Clippy passes with warnings denied.
- Complete release workspace build passes. Both the new formation-recovery
  fixture and the existing startup/journal fixture pass against the final
  release helper, including rejection of provenance for a different parent.

Commands:

```sh
sh crates/linuxdrop-network/tests/run-p2p-host.sh
cargo test --locked -p linuxdrop-netd -p linuxdrop-daemon
sh crates/linuxdrop-netd/tests/run-formation-recovery.sh /absolute/path/linuxdrop-netd
```

Limits remain explicit: an already formed group without verifiable ownership,
a changed supplicant instance, or an old record without provenance is retained.
These cases do not authorize speculative removal of foreign resources. Broader
helper/channel/hardware-loss acceptance and physical Android interoperability
remain in the completion ledger. The live demo was not restarted.
