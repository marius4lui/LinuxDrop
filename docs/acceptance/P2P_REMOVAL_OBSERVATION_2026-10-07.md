# Observe P2P removal before releasing the radio - 2026-10-07

The official [wpa_supplicant D-Bus API](https://w1.fi/wpa_supplicant/devel/dbus.html)
separates Cancel (ongoing formation), Disconnect (a formed group on its group
interface), and root Interfaces/InterfaceRemoved (interface inventory/lifecycle).
LinuxDrop previously settled a group guard immediately after Disconnect returned.

Cleanup now queries the original unique owner's live Interfaces property with
property caching disabled and waits up to five seconds for the exact newly
created group interface object to disappear. It never follows a replacement
well-known service owner. Failure to observe removal becomes the existing typed
cleanup failure, retaining the group identity for recovery. Existing ten-second
operation bounds still cover the D-Bus cleanup sequence.

The privileged helper additionally waits up to five seconds for the corresponding
kernel interface path to disappear after supplicant confirmation. A timeout does
not publish a freed radio reservation. This closes the method-reply/observed-
removal gap for known group identities; it does not identify a group whose
formation event was lost or malformed.

Evidence:

- Four private-bus supplicant tests passed. The added regression returns a
  successful Disconnect while deliberately retaining the interface. Its receipt
  remains pending and completes only after the interface is removed from the
  live inventory without emitting PropertiesChanged. Existing owner-replacement,
  cancellation-race, rejected-disconnect and held-receipt cases still pass.
- Helper binary suite: 20 passed; two namespace-only tests ignored by that command.
- Network/helper all-target Clippy passed with warnings denied.

The private-bus fixture models supplicant inventory, not a physical driver. The
new kernel disappearance wait has not been exercised against a real radio.
Unidentified formation outcomes, pre-marker crash recovery and booted systemd/
polkit acceptance remain separate requirements. Ubuntu DEB revision 33 predates
this source change; its earlier verification is not reused for this new code.
