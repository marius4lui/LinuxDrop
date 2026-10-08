# AirDrop Bluetooth recovery

The optional Apple wake advertisement now has an independent worker. Initial
unavailability, controller power loss and bluetoothd replacement retry after five
seconds. The backend command loop, incoming consent and AWDL transfers do not
wait for Bluetooth registration. Explicit controller selection never falls back
to another radio. Automatic selection is resolved again on each attempt.

Loss and recovery update backend status. The previous registration is removed
before retrying; shutdown drains registration and acknowledged removal. A cleanup
failure remains visible and prevents duplicate registrations. Existing shared
capacity checks still refuse to evict another application's advertisements.

Validation: six AirDrop library tests passed, including real TLS send/receive,
consent and collision handling. The private BlueZ regression passed initial
unavailability, power recovery, service owner replacement, explicit selection and
held-removal shutdown. AirDrop all-target Clippy passed with warnings denied.

Run the focused hardware-free regression from the repository root:

```sh
sh crates/linuxdrop-airdrop/tests/run-bluetooth-recovery.sh
```

This does not certify Apple-device interoperability or fair cross-protocol
advertising windows on single-slot controllers. Those remain separate work.
