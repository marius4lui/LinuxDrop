# Scanner daemon generation safety

The private BlueZ regression reproduced an old scanner's StopDiscovery reaching
a replacement daemon. The scanner's bluez-async session now pins inventory,
adapter, GATT client calls and signal subscriptions to the unique owner captured
at construction. A stale session cannot start discovery after owner replacement;
power is rechecked before setting its filter or starting a scan. Cleanup stays
with the original owner. If it has disconnected, the bus confirms that its unique
name no longer exists and cleanup succeeds without touching a replacement.
Unconfirmed cleanup on a still-live owner continues to fail visibly.

The test retains the old process while a new selected-controller scanner runs,
then cancels the old scan and verifies the new scan is untouched. A not-yet-run
old session is refused. A third daemon covers actual connection loss, and a
separate case powers off the controller between construction and start.
Owner lookup and IO run together before spawning a persistent IO worker, so
constructor cancellation cannot leak that worker. No upstream code was copied.

Validation: both private-bus suites passed (advertisement lifecycle and actual
Quick Share actors, including the scanner). Quick Share all-target Clippy and
bluez-async library Clippy passed with warnings denied. No live demo was modified.

This establishes generation-safe scanning and explicit reconstruction, not full
automatic recovery of scanner/GATT/L2CAP roles. Those remaining rows stay open.
No physical radio acceptance is claimed. References:
[D-Bus unique names](https://dbus.freedesktop.org/doc/dbus-specification.html#message-bus-names),
[BlueZ Adapter API](https://bluez.readthedocs.io/en/latest/adapter-api/).
