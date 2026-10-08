# Channel mutation and retirement - 2026-10-07

SetChannel previously held State through inventory and the external iw command.
It now checks socket ownership, lease kind, revocation and pending producers
before inventory, then rechecks them after the asynchronous scan. Only a free
monitor lease can change channel; direct reservations and AWDL helper-owned
channel schedules remain rejected. Existing ownership and regulatory checks are
preserved and the kernel owner marker is rechecked before the command.

A persistent radio producer owns the mutation. Status reads remain responsive,
concurrent mutations are rejected, and EOF/shutdown requests cancellation without
dropping a running command. Retirement waits for command completion and journal
publication, then reads the final channel. Successful mutation does not reattach
a lease already selected for retirement. A failed command or journal publication
revokes the lease and the caller requests the existing cleanup receipt; failure
retains the reservation and its recovery error. Completed concurrent retirement
also removes the caller's stale ownership entry.

Verification: helper binary tests 20 passed / two namespace-only tests ignored;
all-target helper Clippy passed. The new held-command fixture verifies responsive
Status, rejection of a second SetChannel, incomplete retirement while the command
is held, and retirement observing the new channel. Driver-error and journal-error
fixtures retain recovery-only ownership. The existing revoked/direct reservation
checks still pass. No physical channel switch is implied by these injected
coordination tests.

The previous Ubuntu DEB revision 33 predates this change. A subsequent package or
CI artifact must be used to exercise this source. Pre-marker crash recovery and
unidentified/ambiguous P2P formation outcomes remain separately tracked.
