# Protocol completion matrix

Audit date: 2026-10-06. Checked boxes mean software implemented and locally exercised where practical; physical-device acceptance is tracked separately. Unchecked rows are active implementation work, not scope exclusions.

## LocalSend v2.2

- [x] HTTPS send/receive, certificate fingerprint pinning and TLS signatures.
- [x] UDP announcement, register, info, prepare-upload, upload, cancel.
- [x] Explicit consent, multiple/empty files, exact lengths, checksum validation with 422, collision-safe publication, cancellation and session expiry.
- [x] Reverse HTTP offer with browser page, prepare-download, download, PIN, expiry and per-source session binding.
- [x] Regression: hostile oversized response/unknown file IDs/premature HTTP success.
- [x] Advertise protocol 2.2 consistently; native partial-acceptance tokens and sender partial-success accounting.
- [x] Per-request destination and collision policy using shared ReceiveOptions/storage policy.
- [x] Upload PIN policy and sender PIN challenge/retry with bounded attempts (401/429).
- [ ] Metadata timestamps preserved safely when present; unknown optional fields remain forward-compatible.
- [x] HTTP registration reply to multicast announcement, with UDP fallback; interface-scoped multicast and LAN-only reachability policy.
- [x] Allowed network interfaces, VPN/virtual exclusions, upload request rate limits, shared bandwidth policy.
- [x] Live IPv4 address/interface reconciliation with listener and discovery retirement, stale-peer removal and preservation of connections on unaffected interfaces.
- [ ] Download API client usable through explicit peer offers (server implementation already present).

## Quick Share / Nearby Share (one backend)

- [x] mDNS discovery, real TCP UKEY2 and paired-key negotiation, matching SAS and explicit consent in both directions.
- [x] Encrypted file payloads, empty-file framing, private staging, exact length validation, atomic publication.
- [x] BlueZ BLE advertisements, GATT/L2CAP discovery/session transport and negotiated bandwidth upgrades.
- [x] SSID/password-based peer group joining and sender-hosted network credentials; dedicated disconnected interface required.
- [x] Missing BlueZ preserves LAN functionality; task failures and cancellation produce terminal states; mdns lifecycle cleanup.
- [x] Stable configurable IPv4 LAN listener port and interface-filtered advertisement/discovery/listening; exact local-address binds and source-bound outgoing connections.
- [ ] Live LAN address/interface changes must reconcile listeners and mDNS records without requiring a manual backend restart; IPv6 LAN/address candidates remain separate work.
- [ ] Explicit Bluetooth controller across every scanner/advertiser/GATT/L2CAP path; cooperate with AirDrop advertisement capacity.
- [x] Selected destination/files/collision policy; the UI explains that only publication is selective for bundle-based protocols.
- [x] Payload bandwidth limit shared with all other backends and download offers; waits preserve cancellation and do not delay consent metadata.
- [ ] WPS PIN/device-name Wi-Fi Direct authentication path, P2P group discovery and negotiated group connection.
- [x] Exclusive hardware lease and transfer semaphore for radio-changing upgrades, unique owned NetworkManager profiles and cancellation cleanup; isolated D-Bus lifecycle test passed.
- [ ] Protocol metadata and upgrade failure regression tests beyond existing TCP handshake simulator.

Google's wire schema distinguishes password-based joining from device-name discovery. `device_name` is field 9; field 8 is an optional PIN reserved for future use with the device-name mode, not a requirement that every such offer includes a PIN. The engine now decodes these fields and routes empty-SSID offers through the reserved supplicant helper, with PBC for an empty PIN. Discovery, group identity, cancellation, DHCP and cleanup paths exist. Negotiation/role advertisement, native group-owner behavior and IPv6-only/address-candidate support still require implementation and protocol simulation before this row is complete. Password-based joining and an ordinary NetworkManager AP are not labelled complete Wi-Fi Direct conformance.

## AirDrop / AWDL

- [x] Leased AWDL monitor/TAP helper, regulatory frequency policy, interface-scoped HTTPS/mDNS.
- [x] Discover/Ask/Upload in both directions, explicit consent, source-bound single-use offers, self-signed TLS with per-transfer certificate pinning.
- [x] Streaming dvzip/CPIO, bounded decompression, exact advertised entries, traversal/symlink/device rejection, empty/multiple files, no-replace publication.
- [x] BLE wake, graceful degradation without Bluetooth, cancellation including disconnect during consent.
- [ ] Explicit Bluetooth controller and shared advertisement resource coordination.
- [x] ReceiveOptions destination/selection/collision policy.
- [x] Incoming/outgoing archive bandwidth uses the daemon's shared payload budget; outgoing sockets remain bound to the leased AWDL interface.
- [ ] Reconnect/channel/hardware-loss integration exercised with helper simulations; no false ready state after lease failure.

## Physical acceptance (software work continues independently)

- [ ] Official LocalSend clients: Android/iOS/Windows/macOS; upload, partial acceptance, PIN, cancel, download offer.
- [ ] Current Pixel and Samsung Quick Share: LAN, BLE, direct SSID/password and WPS modes, both directions, SAS and cancellation.
- [ ] iPhone/macOS AirDrop Everyone: send/receive, BLE wake and multiple radios/bands.
- [ ] Real radio injection, channel hopping, unplug recovery, regulatory constraints and simultaneous Bluetooth roles.

## Primary references inspected

- [LocalSend protocol v2.2](https://github.com/localsend/protocol/blob/main/README.md) and [changelog](https://github.com/localsend/protocol/blob/main/CHANGELOG.md).
- [Google Nearby upgrade credentials, authentication types and medium roles](https://github.com/google/nearby/blob/main/connections/implementation/proto/offline_wire_formats.proto).
- [Google Nearby discovery/connection medium selection](https://github.com/google/nearby/blob/main/connections/implementation/p2p_cluster_pcp_handler.cc).
- [Android Wi-Fi Direct service discovery](https://developer.android.com/develop/connectivity/wifi/nsd-wifi-direct).
- Pinned incorporated code and modifications: [PROTOCOL_SOURCES](../../vendor/PROTOCOL_SOURCES.md), [ADR 0002](../adr/0002-protocol-engines.md).

## Network lifecycle verification, 2026-10-06

Temporary NM profiles use `AddAndActivateConnection2` with `persist=volatile` and
`bind-activation=dbus-client`, unique lease UUID/name/interface identity, no
autoconnect, no default route and no imported DHCP DNS/routes. Credentials travel
through D-Bus, not process arguments. Cleanup validates all ownership fields and
keeps the transfer's radio semaphore until cleanup has finished. This follows the
[NetworkManager API](https://networkmanager.dev/docs/api/latest/gdbus-org.freedesktop.NetworkManager.html).

Long P2P discovery, group formation and DHCP waits release netd's global state
lock. The watchdog distinguishes an owned P2P VIF from a competing network.
A separate authorized cancellation connection cancels the operation while the
original daemon worker drains its response; socket loss also cancels formation.
The DHCP hook accepts only the marked group interface and an ordinary IPv4
address/subnet, never offered DNS/default routes. Debian, RPM and Arch dependencies
now include the BusyBox DHCP client; the Ubuntu applet was locally verified.

Current checks: 34 workspace tests passed; Clippy with warnings denied passed;
the real D-Bus/HTTPS daemon integration passed. The additional private-bus NM
test passed activation, serialization, cancellation while waiting for an address,
volatile cleanup, refusal of an active adapter and refusal to delete a foreign
profile even with a matching UUID. Two DHCP-hook validation tests passed.
`sh crates/linuxdrop-network/tests/run-nm-lifecycle.sh` creates its own bus; the
default workspace run deliberately skips that environment-dependent test.

These checks do not emulate driver group formation or prove Android/Windows P2P
interoperability. New release artifacts have not yet been rebuilt. Remaining
software items above stay in the active full implementation goal.

## Transfer policy integration, 2026-10-06

The daemon now provides one payload budget to LocalSend, Quick Share, AirDrop and
reverse-download offers. Both directions and concurrent requests debit that same
budget; handshake/control traffic is excluded. Backend restart revokes the old
download offer before installing the changed network/bandwidth policy. Standalone
backend APIs retain independently scoped budgets for protocol tools and tests.

Quick Share's TCP listeners bind only selected local IPv4 addresses. Discovery
probes, initial TCP sends and LAN upgrades select and bind an allowed source
interface and reject peers outside its local subnet. mDNS interface selection is
paired with explicit service addresses: the pinned mDNS library's `addr_auto`
registration path imports all host addresses independently of interface selection,
so it is deliberately disabled. The selected BlueZ controller now also reaches
the outbound BLE connector. Missing LAN access no longer reports LAN as active.

AirDrop's outbound IPv6 sockets use the interface index supplied by the leased
AWDL discovery context, bind its source address/device and reject unscoped remote
addresses. Network settings describe LAN selection separately from this dedicated
AirDrop adapter. Download-link listeners bind an enabled LAN address; when none
exists, the request fails before consuming the user's file draft.

Verification: the workspace run passed 37 tests (native-display and private-NM-bus
tests remain explicit runners). Real TLS/UKEY2 payload tests now exercise actual
throttling in both directions; parallel reverse downloads share their supplied
budget. Targeted tests cover matching versus unrelated cancellation, forbidden
LAN routes, exact listener and mDNS address sets, and unscoped AirDrop addresses.
The LAN socket test also passed as the unprivileged Ubuntu user. The daemon's
D-Bus/HTTPS integration passed including repeated refusal of a download link on
an excluded network while retaining the prepared draft.

Still open after that pass: LocalSend multicast interface scoping and HTTP-first discovery replies,
live network reconfiguration, reverse-download client, protocol metadata/IPv6/P2P
completion, and the remaining desktop/distribution acceptance. These are software
tasks in the full goal, not physical-device exceptions.

## LocalSend discovery and network changes, 2026-10-06

Announcements now receive a bounded HTTPS/HTTP registration reply first; failure
falls back to a non-announcing UDP response. Certificate pinning also applies to
registration. Discovery responses have a three-second timeout, an eight-request
concurrency bound per interface, bounded datagrams and per-source rate limits.
No response body is downloaded merely to acknowledge registration. Hidden state
suppresses new replies and fallback, and backend removal cancels pending replies.

Every IPv4 multicast socket joins and sends on its selected address and uses
`SO_BINDTODEVICE` plus `IP_MULTICAST_ALL=0`. TCP listeners bind both the selected
local address and interface. Off-subnet, unspecified, multicast and broadcast
peer targets are rejected; outgoing transfers no longer fall back to an arbitrary
adapter. Linux loopback interface flags are preserved even when an address on
that interface is outside 127/8, as with WSL's local DNS alias.

A three-second network watcher reconciles HTTP listeners and multicast workers,
retries newly available interfaces, removes stale peers and updates readiness.
Connections on unchanged interfaces survive reconciliation. Removed interfaces
retire their discovery tasks and drain their listener connections with a bounded
shutdown. This does not restart the other protocols or the visible application.

The workspace passed 40 tests and Clippy with warnings denied. The rebuilt
daemon passed its real D-Bus/HTTPS consent and settings integration. LocalSend's
ten tests also passed as the unprivileged Ubuntu user. New checks use actual UDP/TLS/TCP sockets for HTTP-first
registration, fingerprint-failure fallback, hidden replies and address addition,
removal, recovery and connection continuity. Interface changes are supplied to
the same reconciler used by the watcher, without mutating the host's network.
These checks do not replace interoperability tests with official clients or real
adapter hotplug. Quick Share's live listener/mDNS reconciliation remains open,
along with the other unchecked software items above. Release packages have not
yet been rebuilt with these changes.
