# Supplicant group cleanup acknowledgement - 2026-10-07

A successfully created P2P group has a guard until netd journals its identity.
Previously, failure after guard creation (for example an unapproved channel or
rejected WPS setup) dropped the guard and returned while its asynchronous
Disconnect was still running. Its outcome was silently discarded.

Each guard now exposes a cloneable settlement receipt. Drop starts the existing
owner-pinned, bounded cleanup and publishes its outcome. Dropping a receipt
waiter does not cancel that task. Durable journal handoff settles the receipt
without disconnecting: ownership has moved to netd. Missing runtime, stopped
worker, timeout and Disconnect errors produce failure instead of success.

The client and group-owner creation paths retain this receipt when a guard is
created and await it before returning an error. This closes their ordinary
post-guard failure return race. It does not yet join every abandoned producer
from netd's full lease-retirement worker; that integration and pre-guard/late
GroupStarted recovery remain in the helper lifecycle checklist.

Verification in LinuxDrop-Dev:

- Three private-bus supplicant scenarios passed using
  `crates/linuxdrop-network/tests/run-p2p-host.sh`.
- The new scenario holds the real fixture Disconnect reply and proves failed
  creation stays pending until cleanup completes. It covers cancelled-waiter
  independence, normal guard-drop acknowledgement, journal handoff without
  disconnect, and propagation of rejected teardown.
- Existing owner-replacement, client discovery cancellation, late GroupStarted,
  host credentials and WPS-failure scenarios still pass.
- Network and netd all-target Clippy with warnings denied passed.

The tests use a private D-Bus service and no physical radio. They do not establish
installed systemd/polkit or physical Apple/Android interoperability. No package
is relabelled as including this change before rebuilding its dependent binaries.
