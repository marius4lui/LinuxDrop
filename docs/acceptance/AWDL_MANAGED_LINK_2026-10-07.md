# Managed AWDL link readiness and failures, 2026-10-07

Source review found three real gaps between Filin and netd:

1. A TAP existed before monitor tuning/raw-socket initialization finished, but
   netd treated existence as successful startup.
2. A failed runtime channel change only logged a warning after advancing the
   logical AWDL schedule, allowing traffic to continue on the wrong channel.
3. Filin's standalone retry loop could reopen an interface by name after a
   failure instead of returning recovery control to the privileged lease owner.

Netd now starts Filin in managed-lease mode. The process writes exactly
`LINUXDROP_AWDL_READY_V1` plus newline after successful link initialization.
Logs use stderr. Netd reads a fixed-size receipt with cancellation and a
three-second deadline, checks child liveness, TAP presence and the monitor's
ownership marker, and reaps failed startup before returning. A pre-existing TAP
name is refused before starting Filin. Mere interface existence is insufficient.

In managed mode Filin exits on link startup/runtime errors. Failed channel
changes return before further traffic, and the runtime checks both monitor and
TAP liveness/ifindex each second. The netd watchdog also retires monitor leases
when ownership or TAP presence disappears. Recovery remains under netd; the
standalone Filin mode retains its upstream retry behavior.

Limited radios also constrain their own advertised channel sequence to the
lease's allowed frequencies before scheduling hops. Unsupported master slots
fall back to the configured allowed anchor; peer sequences remain unchanged so
transmit-overlap decisions retain the peer's actual availability. Park selection
and transfer pins obey the same policy. A real tuning failure on an allowed
channel still terminates the link. This preserves a channel-6 software path for
2.4-GHz-only adapters without claiming tested AR9271 interoperability.

Focused evidence:

- 23 ordinary netd tests pass. The new readiness test exercises actual child
  pipes, a valid receipt, malformed output, early process exit and cancellation
  while an interface already exists. Existing producer/retirement checks remain.
- A separate ignored test runs the **actual release Filin binary** with the
  production netd receipt reader in a private network namespace. A real dummy
  link permits TAP creation but rejects actual nl80211 channel configuration.
  Filin exits with the channel error, netd does not publish ready, and the
  nonpersistent TAP disappears. This is a software fault-path check, not radio
  emulation or successful AWDL interoperability.
- All 25 AWDL state tests pass, including restricted-band master adoption,
  unchanged peer availability, park/pin constraints and dual-band preservation.
- Netd all-target Clippy passes with warnings denied. CI locates exact netd
  test and Filin artifacts and runs the same isolated fault-path check.

Reproduce the actual-process case:

```sh
sh crates/linuxdrop-netd/tests/run-awdl-link.sh /path/netd-test-binary /path/filin
```

The existing real AirDrop listener/discovery actor reconnect check remains in
[helper lifetime acceptance](AIRDROP_HELPER_LIFETIME_2026-10-07.md). The broader
helper/watchdog/channel-loss chain is now exercised by the separate
[combined integration fixture](AWDL_WATCHDOG_2026-10-07.md). Successful
radio injection, channel hopping with Apple peers and physical unplug remain
hardware acceptance. The live demo session was not modified.
