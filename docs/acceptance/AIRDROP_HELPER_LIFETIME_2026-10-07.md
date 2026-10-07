# AirDrop helper lifetime and reconnect

The daemon now runs separate health workers for its AirDrop and Quick Share
leases. Each checks every five seconds, with missed ticks skipped. A Wi-Fi
Direct operation holding its helper connection cannot delay AirDrop's next
health check. Started framed requests still finish before shutdown; the existing
five-second status timeout retires a failed connection instead of reusing an
uncertain response stream.

Verified on 2026-10-07:

- Six daemon helper regressions passed. The new concurrency case keeps the
  Quick Share socket locked while AirDrop changes from a healthy lease to a
  revoked lease. The actual watcher removes the AirDrop command route and
  reports an error on its next poll, while Quick Share remains blocked.
- An isolated kernel network fixture starts the actual AirDrop listener and
  mDNS worker on an IPv6 dummy interface. A simulated helper first returns a
  healthy lease, then revokes it. The link is removed, the daemon drains the
  real backend, stale ready events cannot restore readiness, and starting on
  the missing link fails.
- The fixture recreates that interface and address and successfully starts a
  second backend generation. This time helper socket EOF triggers retirement.
  Both generations use the production shutdown receipt. The fixture passed in
  0.07 seconds and is included in the CI isolated-network job.
- Daemon all-target Clippy passed.

Run the ignored network fixture with the built `linuxdropd` test executable:

```sh
sh crates/linuxdrop-daemon/tests/run-awdl-lifecycle.sh /path/to/linuxdropd-test-binary
```

The wrapper creates a fresh network namespace. The fixture verifies it differs
from PID 1's namespace before creating or deleting any interface. It uses a Unix
socket pair for helper responses and disables BLE. It does not start the AWDL
radio engine, acquire a real helper lease, exercise regulatory/channel changes,
or prove Apple-device interoperability. Actual helper channel/hardware-loss
integration and physical radio acceptance remain separately open.
