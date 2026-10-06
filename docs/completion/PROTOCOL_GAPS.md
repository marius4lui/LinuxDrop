# Protocol completion matrix

Audit date: 2026-10-06. Checked boxes mean software implemented and locally exercised where practical; physical-device acceptance is tracked separately. Unchecked rows are active implementation work, not scope exclusions.

## LocalSend v2.2

- [x] HTTPS send/receive, certificate fingerprint pinning and TLS signatures.
- [x] UDP announcement, register, info, prepare-upload, upload, cancel.
- [x] Explicit consent, multiple/empty files, exact lengths, checksum validation with 422, collision-safe publication, cancellation and session expiry.
- [x] Reverse HTTP offer with browser page, prepare-download, download, PIN, expiry and per-source session binding.
- [x] Regression: hostile oversized response/unknown file IDs/premature HTTP success.
- [ ] Advertise protocol 2.2 consistently; native partial-acceptance tokens and sender partial-success accounting.
- [ ] Per-request destination and collision policy using shared ReceiveOptions/storage policy.
- [ ] Upload PIN policy and sender PIN challenge/retry with bounded attempts (401/429).
- [ ] Metadata timestamps preserved safely when present; unknown optional fields remain forward-compatible.
- [ ] HTTP registration reply to multicast announcement, with UDP fallback; interface-scoped multicast and LAN-only reachability policy.
- [ ] Allowed network interfaces, VPN/virtual exclusions, upload request rate limits, bandwidth policy.
- [ ] Download API client usable through explicit peer offers (server implementation already present).

## Quick Share / Nearby Share (one backend)

- [x] mDNS discovery, real TCP UKEY2 and paired-key negotiation, matching SAS and explicit consent in both directions.
- [x] Encrypted file payloads, empty-file framing, private staging, exact length validation, atomic publication.
- [x] BlueZ BLE advertisements, GATT/L2CAP discovery/session transport and negotiated bandwidth upgrades.
- [x] SSID/password-based peer group joining and sender-hosted network credentials; dedicated disconnected interface required.
- [x] Missing BlueZ preserves LAN functionality; task failures and cancellation produce terminal states; mdns lifecycle cleanup.
- [ ] Stable configurable LAN listener port and interface-filtered advertisement/discovery/listening.
- [ ] Explicit Bluetooth controller across every scanner/advertiser/GATT/L2CAP path; cooperate with AirDrop advertisement capacity.
- [ ] Selected destination/files/collision policy and bandwidth limits.
- [ ] WPS PIN/device-name Wi-Fi Direct authentication path, P2P group discovery and negotiated group connection.
- [ ] Exclusive hardware lease for all radio-changing upgrades, unique owned NetworkManager profiles and cancellation-safe cleanup.
- [ ] Protocol metadata and upgrade failure regression tests beyond existing TCP handshake simulator.

Google's current implementation uses Wi-Fi Direct as an upgraded medium after endpoint discovery, so a generic standalone Wi-Fi Direct scan does not itself discover Quick Share recipients. However its newer empty-SSID credentials carry a P2P device name and WPS PIN; this is a real missing connection mode in the current vendored engine and is being implemented. Password-based group joining alone is not labelled complete Wi-Fi Direct conformance.

## AirDrop / AWDL

- [x] Leased AWDL monitor/TAP helper, regulatory frequency policy, interface-scoped HTTPS/mDNS.
- [x] Discover/Ask/Upload in both directions, explicit consent, source-bound single-use offers, self-signed TLS with per-transfer certificate pinning.
- [x] Streaming dvzip/CPIO, bounded decompression, exact advertised entries, traversal/symlink/device rejection, empty/multiple files, no-replace publication.
- [x] BLE wake, graceful degradation without Bluetooth, cancellation including disconnect during consent.
- [ ] Explicit Bluetooth controller and shared advertisement resource coordination.
- [ ] ReceiveOptions destination/selection/collision policy and bandwidth limit.
- [ ] Reconnect/channel/hardware-loss integration exercised with helper simulations; no false ready state after lease failure.

## Physical acceptance (software work continues independently)

- [ ] Official LocalSend clients: Android/iOS/Windows/macOS; upload, partial acceptance, PIN, cancel, download offer.
- [ ] Current Pixel and Samsung Quick Share: LAN, BLE, direct SSID/password and WPS modes, both directions, SAS and cancellation.
- [ ] iPhone/macOS AirDrop Everyone: send/receive, BLE wake and multiple radios/bands.
- [ ] Real radio injection, channel hopping, unplug recovery, regulatory constraints and simultaneous Bluetooth roles.

## Primary references inspected

- [LocalSend protocol v2.2](https://github.com/localsend/protocol/blob/main/README.md) and [changelog](https://github.com/localsend/protocol/blob/main/CHANGELOG.md).
- [Google Nearby Wi-Fi Direct bandwidth-upgrade handler](https://github.com/google/nearby/blob/main/connections/implementation/mediums/wifi_direct_bwu_handler.cc).
- [Google Nearby discovery/connection medium selection](https://github.com/google/nearby/blob/main/connections/implementation/p2p_cluster_pcp_handler.cc).
- [Android Wi-Fi Direct service discovery](https://developer.android.com/develop/connectivity/wifi/nsd-wifi-direct).
- Pinned incorporated code and modifications: [PROTOCOL_SOURCES](../../vendor/PROTOCOL_SOURCES.md), [ADR 0002](../adr/0002-protocol-engines.md).
