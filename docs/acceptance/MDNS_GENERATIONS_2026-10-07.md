# mDNS generations during network changes

The pinned mDNS implementation appends interface-selection rules on every
enable/disable call. LinuxDrop previously reused that history indefinitely in
both the advertiser and discovery worker. Each changed interface snapshot now
retires the old worker with an acknowledged shutdown before creating a new one.
Snapshots which change only diagnostics do not rebuild discovery.

The advertiser withdraws its service first and registers the current explicit
address set on the new worker. Its identity, visibility and temporary-visibility
deadline remain owned by the backend. Discovery drains cancelled TCP probes,
discards its old event stream and withdraws cached peers before browsing again.
Old peers cannot outlive the resolver which owned their TTL/removal events.
The constructor's actual interface set is retained, covering changes before run
starts. A failed replacement does not double-stop an already retired worker.

Validation used a private kernel network namespace, never host interfaces:

- The running engine followed IPv4 removal, IPv6-only readiness, new addresses
  and link down/up without backend-failure events.
- A separate test performed 32 alternating interface snapshots. Each transition
  removed both prior mDNS thread IDs, left exactly two replacement workers and
  retained a bounded descriptor count. Shutdown left no mDNS workers.
- A constructor-to-run snapshot change was included in the same regression.
- The real TCP listener test preserved an accepted connection across interface
  reconciliation and excluded failed/loopback binds from advertised metadata.
- Quick Share all-target Clippy passed with warnings denied.

The repeated transitions are accelerated; this is resource-lifecycle acceptance,
not a days-long soak or an adversarial multicast-flood bound. Official clients,
real access-point roaming and physical radio acceptance remain separate.
