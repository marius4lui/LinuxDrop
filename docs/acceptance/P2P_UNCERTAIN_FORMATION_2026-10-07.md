# Uncertain Wi-Fi Direct formation - 2026-10-07

The previous error path ignored Cancel failures, malformed late GroupStarted
messages and failed identity lookups. A helper could consequently return a
generic connection failure and release a radio while supplicant still owned an
unidentified group. The same gap existed if the helper stopped before receiving
the first group identity.

Both client and group-owner formation now track whether a mutating request was
submitted. On error, an existing group guard must finish its cleanup. Otherwise
the original pinned supplicant owner receives Cancel, and a late group is
identified and disconnected with the existing removal acknowledgement. With no
late signal, Cancel must succeed. In either case a fresh, uncached interface
inventory must contain no newly introduced object. Silence for 250 ms is no
longer sufficient evidence of cleanup. An unknown outcome is a typed error;
known-group cleanup failures retain their more precise identity.

This distinction follows the upstream [supplicant D-Bus API](https://w1.fi/wpa_supplicant/devel/dbus.html):
Cancel stops ongoing formation; Disconnect terminates an existing group.
Neither a missing event nor an error reply establishes resource removal.

Before starting formation, netd durably records `p2p_pending` in its existing
lease journal. It clears that intent only after settlement or replacement with
a known group identity. Unknown outcomes revoke the usable lease and retain
the physical-radio reservation. A helper restart cannot erase it. Recovery
status reports uncertain ownership rather than claiming that retry can remove
an identified resource. Legacy journals default this additive field to false.

Focused validation:

- Five private-bus supplicant tests pass, including malformed initial/late
  signals, rejected Cancel, successful cancellation with no group, known-group
  removal acknowledgement and service-owner replacement.
- 21 helper tests pass, including durable-intent serialization, retained
  reservation on failed cleanup and legacy journal compatibility. Both additional
  isolated kernel address/DHCP tests pass. Targeted network/helper/daemon Clippy
  passes and the complete release workspace builds.
- The real release helper in private mount/network namespaces retains unknown
  formation across two actual process starts and reports it as unverified. A
  synthetic old-boot journal clears successfully; this is not a physical reboot
  test. Existing malformed journal and ownership-marker recovery probes pass.

Follow-up: [durable formation provenance and current-boot recovery](P2P_FORMATION_RECOVERY_2026-10-07.md)
now allow the original supplicant instance to acknowledge cancellation after a
helper restart. The following limitation still applies to records without that
provenance and to unidentified already formed groups or changed service owners.

Earlier limitation: an unidentified formation retained after a helper crash
cannot yet be automatically reconciled during the same system boot. A system
reboot establishes that the old kernel operation is gone. Implementing safe
same-boot reconciliation needs more durable provenance; this change deliberately
does not infer ownership from an interface name or delete an unrelated group.
Physical radio/Android interoperability is still separate acceptance.
