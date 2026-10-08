# ADR 0002: protocol engines and safety boundaries

Date: 2026-10-06. Decision: integrate pinned Rust protocol implementations behind LinuxDrop's command/event domain. Nearby Share and Quick Share are one backend.

## Evaluation

| Candidate | Inspected snapshot | Result |
|---|---|---|
| RQuickShare | `378d8ae969941bee4bf60ad34ac9cf8bb7005eb7` | Reusable RQS Rust core with LAN discovery and UKEY2, but fewer direct/BLE transport paths than its evaluated fork. |
| open-quickshare | `5a31145163ee22ab9cf1c7d3dffd74355febe93f` | Selected for LAN, BlueZ BLE GATT/L2CAP, encrypted payload and negotiated Wi-Fi bandwidth-upgrade implementations. Audited reachable integration boundaries and patched unsafe defaults. |
| opendrop-rs | `dccc798e244363eb92d35e3c52e9a913188dda91` | Selected for Rust AWDL radio/link implementation (Filin) and AirDrop plist/TLS/mDNS building blocks. Its auto-accept server and permissive archive salvage functions are deliberately outside our runtime. |
| airdrop-mt7921 | `9f22b77be06a325e66cf600910a494efa71d76d7` | Useful iOS 26 and BLE advertisement reference. Its Python/shell application and passwordless sudo installation are not adopted. Its reported device compatibility remains upstream evidence. |

Source links and licenses are in [PROTOCOL_SOURCES](../../vendor/PROTOCOL_SOURCES.md). LinuxDrop is GPL-3.0-only. No upstream README is treated as a completed LinuxDrop interoperability test.

## Quick Share

`linuxdrop-quickshare::start(Config, events)` owns one RQS engine, discovery and a command actor. It translates all upstream events to core Peer/Transfer values. No upstream object or network-controlled path is exposed over D-Bus. Runtime settings cover receiver name, port, visibility, BLE, receive size/count, and an optional dedicated interface for direct upgrades.

SAS confirmation is bound to the authenticated UKEY2 session metadata. Both sender and receiver must explicitly confirm; the UI receives `verification` state and the four-digit code. This code authenticates the current connection when users compare both screens, not a durable device identity. Non-file incoming payloads are rejected pending a separate explicit product flow.

The direct transport negotiates Wi-Fi LAN / Wi-Fi Direct or hotspot credentials over the encrypted BLE link. NetworkManager joins or creates the offered group on an explicitly selected disconnected adapter. This is not a separate universal Wi-Fi P2P discovery implementation. A single active Internet adapter is never automatically repurposed. A large BLE-only send that cannot upgrade fails with an actionable transport limitation instead of pretending to finish.

Received files remain in a private random staging directory until the protocol reports all advertised lengths complete; publication uses the shared directory-descriptor ReceiveStore. Each session is independent. A failed or rejected job cannot overwrite a terminal UI state. File data arriving before consent is an error. Background task lifetime is tied to backend shutdown.

A missing or powered-off BlueZ controller leaves LAN transfers available with an explicit Bluetooth diagnostic. TCP or mDNS task failures stop the backend and terminate outstanding transfers; Bluetooth advertisement failures degrade Bluetooth availability without masking LAN readiness. mDNS service threads are released on every stop or failed startup.

## AirDrop

The privileged network helper leases an idle radio, creates an owned monitor interface, and launches Filin with an owned TAP name. All radio frequency changes pass the helper's regulatory allowlist. The user daemon never runs as root. Hardware release terminates Filin and removes owned interfaces.

`linuxdrop-airdrop::start(Config, events)` requires that leased AWDL interface, derives its IPv6 scope, binds HTTPS only to that address, and restricts mDNS to that interface. BlueZ registers a Continuity advertisement when wake is enabled; it respects an already powered-off controller and releases the advertisement on stop.

AirDrop Everyone uses self-signed certificates and does not provide a contacts identity. Outgoing TLS proof of possession is verified and the certificate is pinned for Discover, Ask and Upload. Incoming uploads are bound to a consented source address, have a single-use offer and expiry, and require explicit consent; unsolicited uploads fail. The protocol does not provide a LocalSend-style upload token, so this source-address binding is a documented property of Everyone mode, not a claimed strong account identity.

Uploads stream to private anonymous temporary files. dvzip frames are independently inflated with a strict output budget; CPIO requires a trailer and exact advertised entries, rejects traversal, directories other than the leading dot entry, and every symlink/device entry. Source and destination memory use is bounded. Sending prepares a private disk archive before requesting the remote transfer; insufficient temporary disk space is an error. Some Apple Ask messages omit file sizes; these receive under the configured global byte limit and only acquire final byte totals during archive validation.

## Evidence and remaining acceptance

Verified on Ubuntu 24.04 under WSL using native Linux Rust tooling:

- Both adapter crates compile and pass Clippy on all own targets with warnings denied.
- Real TCP Quick Share UKEY2/paired-key handshake, equal SAS codes, explicit consent at both ends and byte-identical transfer. A fragmented first frame with interleaved UI commands is part of the test.
- Real TLS AirDrop Discover/Ask/Upload, multi-file and empty-file transfer, destination collision preservation and explicit consent before files appear.
- AirDrop sender cancellation while the recipient is deciding closes the TLS connection, removes pending consent, and emits a terminal failure instead of leaving a stuck request.
- Quick Share startup with Bluetooth requested and no BlueZ service retains LAN readiness; an occupied TCP port fails without a false ready event.
- Strict archive traversal/truncation rejection and decompression-budget enforcement.
- Filin's frequency policy rejects forbidden and malformed policies before any nl80211 operation.

These are protocol loopback/simulator tests, not Android or Apple device interoperability. Pending physical acceptance: Pixel/Samsung Quick Share versions, Android BLE/Wi-Fi upgrade combinations, iPhone/macOS Everyone mode, actual AWDL frame injection/channel behavior, radio hot-unplug recovery and BLE wake behavior. The software contains real paths and reports hardware/backend failures explicitly; the package must retain the experimental AirDrop label until those checks are recorded.

## Reproduction

```sh
cargo test -p linuxdrop-quickshare -p linuxdrop-airdrop --lib
cargo clippy -p linuxdrop-quickshare -p linuxdrop-airdrop --all-targets --no-deps -- -D warnings
cargo test --manifest-path vendor/opendrop-rs/Cargo.toml -p filin-rs linuxdrop_policy_tests --lib
cargo build --manifest-path vendor/opendrop-rs/Cargo.toml -p filin-rs --release
```

System build dependencies: Rust (2024-edition-capable), C compiler, pkg-config, libdbus-1-dev and protobuf-compiler. The radio runtime additionally requires BlueZ, NetworkManager and the separately installed hardened netd service.
