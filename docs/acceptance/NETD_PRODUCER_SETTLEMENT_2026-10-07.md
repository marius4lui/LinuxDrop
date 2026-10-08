# P2P producer retirement acknowledgement - 2026-10-07

A disconnected helper socket previously dropped its group/address producer while
full retirement could already restore and release the radio. Group Drop cleanup
and DHCP shutdown could consequently overlap later use of the same reservation.

The helper now owns a persistent producer supervisor. Dropping a socket requests
cancellation; full retirement waits outside State for the producer receipt and
then reads the final journaled identity. A producer panic retains the reservation
and a failed receipt instead of authorizing reuse. Status remains serviceable
while a producer is deliberately held in the regression fixture.

Group formation and publication failures await the network group's cleanup
receipt. Failed cleanup carries the exact known group identity back to the
helper, which retains it for recovery. Late-event cleanup no longer bypasses
an already-running guard cleanup receipt on error. DHCP address acquisition
checks cancellation internally, and failure or cancellation reaps its child
before the producer settles. A full cleanup queued during address failure owns
the subsequent journaled group teardown. No group credentials are journaled.

The service stop budget is 90 seconds to accommodate bounded producer and group
cleanup phases. This is a configured budget, not a measured physical-radio SLA;
systemd control-group termination and the durable journal remain the fallback.

Verification:

- Helper binary suite: 15 passed, two namespace-only tests explicitly ignored.
  The new held-producer regression proves no restoration before settlement and
  use of the producer's final identity; a synthetic panic cannot release a radio.
- Both ignored tests were then run in private kernel network namespaces: client
  IPv6 readiness and a real waiting DHCP client's cancellation/reaping passed;
  group-owner DHCP passed without advertising a default router or DNS server.
- All three private-bus supplicant scenarios passed, including held cleanup,
  dropped waiters and rejected Disconnect retaining a typed recovery identity.
- Network and helper all-target Clippy passed with warnings denied.

Open implementation boundaries: a failure to identify a late GroupStarted event
or an ambiguous Cancel result still needs explicit recovery semantics. Creation
before durable publication needs crash recovery; a known but unmarked group must
not be deleted by weakening ownership checks. Acquisition rollback still awaits
I/O under State. This pass does not claim booted systemd/polkit or physical-radio
acceptance.
