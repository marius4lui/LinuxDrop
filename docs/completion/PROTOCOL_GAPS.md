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
- [x] Metadata timestamps preserved safely when present; unknown optional fields remain forward-compatible.
- [x] HTTP registration reply to multicast announcement, with UDP fallback; interface-scoped multicast and LAN-only reachability policy.
- [x] Allowed network interfaces, VPN/virtual exclusions, upload request rate limits, shared bandwidth policy.
- [x] Live IPv4 address/interface reconciliation with listener and discovery retirement, stale-peer removal and preservation of connections on unaffected interfaces.
- [x] Download API client usable through explicit local HTTP offers, native link entry, PIN challenge, receive review and selected-file publication.

## Quick Share / Nearby Share (one backend)

- [x] mDNS discovery, real TCP UKEY2 and paired-key negotiation, matching SAS and explicit consent in both directions.
- [x] Encrypted file payloads, empty-file framing, private staging, exact length validation, atomic publication.
- [x] BlueZ BLE advertisements, GATT/L2CAP discovery/session transport and negotiated bandwidth upgrades.
- [x] SSID/password-based peer group joining and sender-hosted network credentials; dedicated disconnected interface required.
- [x] Missing BlueZ preserves LAN functionality; task failures and cancellation produce terminal states; mdns lifecycle cleanup.
- [x] Stable configurable IPv4 LAN listener port and interface-filtered advertisement/discovery/listening; exact local-address binds and source-bound outgoing connections.
- [x] Live IPv4 LAN address/interface changes reconcile listeners, mDNS records, discovery probes and readiness without requiring a manual backend restart.
- [x] IPv6 LAN listeners, scoped discovery endpoints and WIFI_LAN address-candidate offers/selection, including IPv6-only readiness and exact interface binding.
- [x] IPv6 credentials/candidates for direct/hotspot networks; owned-interface binding and IPv6-only group readiness.
- [x] Pin supplicant operations and journaled P2P cleanup to the original unique D-Bus owner/bus; abort owner changes and distinguish revoked leases from healthy reservations.
- [ ] Prolonged mDNS reconfiguration/resource-bound acceptance.
- [ ] Explicit Bluetooth controller across every scanner/advertiser/GATT/L2CAP path; cooperate with AirDrop advertisement capacity.
- Scanner owner replacement now has [private-bus regression evidence](../acceptance/BLUETOOTH_SCANNER_OWNER_2026-10-07.md): old cleanup cannot reach a new daemon, stale starts are rejected, and explicit reconstruction works. Automatic recovery now also covers the listener, receiver, sender and recipient scanner; prolonged resource bounds and cross-protocol airtime remain open.
- The FastInit announcement scanner now has [automatic recovery](../acceptance/BLUETOOTH_SCANNER_RECOVERY_2026-10-07.md) for power, controller and daemon loss, including initial unavailability and deliberate airtime pauses. Receiver and sender supervisors have their own lifecycle fixtures; recipient discovery and pre-connect scans now have [owned cleanup and recovery](../acceptance/BLUETOOTH_RECIPIENT_SCANS_2026-10-07.md).
- [x] Selected destination/files/collision policy; the UI explains that only publication is selective for bundle-based protocols.
- [x] Payload bandwidth limit shared with all other backends and download offers; waits preserve cancellation and do not delay consent metadata.
- [x] Receiver-initiated dynamic role switching with advertised local identity, reserved-radio joining, consent, cancellation and encrypted channel continuity.
- [x] Device-name/PBC Wi-Fi Direct hosting and joining, optional eight-digit PIN joining, P2P discovery and negotiated group connection (software/isolated fixtures).
- [x] Exclusive hardware lease and transfer semaphore for radio-changing upgrades, unique owned NetworkManager profiles and cancellation cleanup; isolated D-Bus lifecycle test passed.
- [x] Protocol metadata and upgrade failure regressions beyond the TCP handshake: role/auth selection, consent, introduction/ACK checks, strict handoff, cancellation and isolated owned-radio cleanup.

Google's wire schema distinguishes password-based joining from device-name discovery. `device_name` is field 9; field 8 is an optional PIN reserved for future use with the device-name mode, not a requirement that every such offer includes a PIN. The engine now decodes these fields and routes empty-SSID offers through the reserved supplicant helper, with PBC for an empty PIN. Discovery, group identity, cancellation, DHCP and cleanup paths exist. Password-based group hosting, capability-based role advertisement and IPv6/address-candidate paths are now implemented and have scoped simulation evidence below. Device-name/PBC hosting now follows peer authentication capabilities and uses the existing supplicant identity. Full physical peer interoperability remains unverified. Receiver-as-client dynamic role switching and supplicant owner-change handling have scoped implementation and simulation evidence below. Password-based joining and an ordinary NetworkManager AP are not labelled complete Wi-Fi Direct conformance.

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

## Quick Share network lifecycle and key decoding, 2026-10-06

One listener owner now publishes the exact bound-interface snapshot used by
mDNS advertisement, discovery and backend readiness. It polls network changes
every three seconds, retains unchanged listeners, retries failed new binds and
removes listeners on retired interfaces. Existing accepted TCP sessions have
independent ownership. mDNS withdraws the previous record before registering
the current explicit address set; loopback aliases and failed binds are omitted.
Visibility is checked again before network-triggered advertisement. Removal
waits are asynchronous and bounded rather than blocking the runtime thread.

Discovery probes run independently of network and cancellation events, with at
most sixteen pending services and sixteen IPv4 candidates per service under one
three-second deadline. Network changes cancel stale probes and prune unreachable
peers. Removed or superseded discovery results cannot resurrect an older peer.
Bluetooth component failures remain visible when LAN connectivity recovers.

The isolated namespace runner exercises an actual running engine while adding
an address and bringing a dummy link down/up, then verifies listener recovery,
absence of component-failure events and bounded shutdown. The normal socket test
also proves that an existing accepted connection survives other listener changes
and that failed binds never enter generated service metadata. These are software
checks; they do not prove remote Android discovery or capture every external
resolver's cache behavior.

The broader test run exposed a separate P-256 interoperability bug: signed wire
coordinates shorter than 32 bytes were concatenated without left-padding, and
oversized coordinates were silently truncated. Both handshake directions now
use one decoder which restores fixed-width coordinates, rejects negative or
oversized integers and verifies the resulting curve point and key type. A fixed
379*G public-point regression covers the short-x/sign-prefixed-y case, invalid
points and truncation attempts; the real UKEY2/SAS/payload exchange passes too.
LocalSend's parallel fixtures now allocate distinct server ports to avoid an
observed race while generating their TLS identities.

Verification: 43 workspace tests, Clippy with warnings denied and the rebuilt
daemon's D-Bus/HTTPS integration passed. The private network-namespace test passed
separately; the normal listener-lifecycle test also passed as the unprivileged
Ubuntu user. Release artifacts remain pending. One identified follow-up is the
pinned mDNS library's append-only interface-selection history: repeated runtime
reconfiguration needs a bounded daemon refresh strategy and an accompanying
long-running discovery/cache check. IPv6 and full P2P negotiation remain active
software work, not device-only acceptance exceptions.

## Native LocalSend download reception, 2026-10-06

`ReceiveDownloadOffer` starts a tracked incoming transfer from an explicitly
entered local HTTP link. The Transfers page exposes the native entry dialog.
Metadata and PIN negotiation precede the existing receive review; only accepted
files are requested, using the chosen destination and collision policy. An
explicit download works while hidden without enabling unsolicited reception.
Parallel limits, blocked-peer decisions, lock-state checks and normal history
and completion handling remain in the daemon. Reverse-only endpoints are not
incorrectly added to the nearby upload-device list.

The client accepts literal on-link addresses under the network policy, binds its
source interface and rejects URL credentials, extra paths/queries/fragments,
website names and redirects. Metadata size, file count, names, lengths and
optional checksums are checked. PIN waits, consent waits, response/stream waits
and cancellation are bounded; payloads use the shared bandwidth budget. Partial
files are removed on failure, and request errors omit URLs containing session
tokens or PINs. HTTP is disclosed in the dialog, as required for browser-style
LocalSend download interoperability; it is not represented as encrypted sharing.

Verification: 46 workspace tests passed, including real HTTP offers with wrong
PIN retry, partial selection, empty files, collisions, traversal, size/checksum
failures and cancellation. The actual daemon D-Bus/HTTPS integration additionally
exercises reverse-download PIN and consent while hidden and verifies saved bytes.
The native GTK regression passed separately; the final rendered 480x600 German
dialog was inspected at `docs/acceptance/ui/completion/download-offer-review.png`.
This pass does not establish official-client or physical-device acceptance, and
the final installation packages still need rebuilding.


## Restart admission follow-up, 2026-10-07

Manual restarts, network-setting updates and USB-triggered retries now reserve a
shared `restarting` state under the same data mutex used to admit transfers.
Startup has that state too. Outgoing sends, incoming download offers and new
reverse offers reject during the transition; draft descriptors remain owned.
Conflicting setting changes cannot queue another restart or persist a new network
configuration midway through startup. New inbound requests rejected by daemon
policy are recorded terminal immediately. The latest visibility/lock state is
reapplied before admission reopens, and GTK disables send/link actions with an
explicit restart explanation.

The actual D-Bus integration covers transition-time rejection of all three entry
points, duplicate restart/network mutation rejection, retained source descriptors,
and refusal to interrupt an existing receive request. Native GTK verifies the
transition controls. This closes admission races, not the entire lifecycle row:
the fixed backend shutdown delay still needs explicit resource-drain completion,
and reverse-offer active stream accounting/helper-loss simulations remain required.

2026-10-07 Bluetooth lifecycle follow-up: explicit unregister acknowledgement,
registration-cancellation cleanup, shared slot admission and dedicated D-Bus owner
teardown are implemented and exercised by the private BlueZ mock. The broader
controller rows remain unchecked until external Release/power-loss and every
scanner/GATT/L2CAP path have their matching software acceptance.


## Helper-loss recovery, 2026-10-07

Daemon helper sockets now retain the exact acquired lease identity and the backend
startup generation. Status for another lease owned by the same UID cannot keep a
lost service ready. Helper I/O failures and timeouts quarantine that generation,
remove its discoverable protocols, fail unfinished transfers with persisted
history/notifications, and retain backend cleanup receipts for restart. Delayed
ready/discovery/progress events cannot revive the failed backend. Actors finishing
initialization after a fault are retired instead of exposed for sending. Generation
advancement and event/fault application share the data lock; stale health results
cannot stop the replacement service. P2P connector instances cannot reuse another
lease. Cleanup runs outside the data lock and outside the P2P request's actor wait.

Passed: nine daemon tests (four new actual Unix-socket/race/cleanup regressions),
all-target daemon/netd Clippy, and the real daemon private D-Bus/HTTPS integration.
The helper-loss tests cover exact versus unrelated same-user leases, pending cleanup,
late initialization, stale results across restart and P2P socket failure. No physical
unplug acceptance or completion of all netd journal/recovery work is inferred.
Final daemon-exit draining remains a separate software item. Installed packages
and the user's live demo have not been replaced by this source revision.


## Confirmed daemon exit and radio release, 2026-10-07

SIGTERM, SIGINT and StopWhenIdle now share admission closure and an explicit
cleanup phase. In-flight constructors are allowed to return their resource-owning
handles; actors completing after stop are retired, and no subsequent backend is
started. The daemon waits for its supervisor and safely ends watcher requests.
Backend actors and link servers drain concurrently with those watchers so a P2P
request cannot deadlock shutdown behind a health check. It keeps consuming bounded
event channels throughout cleanup, then waits for all event forwarders and drains
the remaining queue before persisting final transfer history. Nonterminal leftovers
are recorded as interrupted, while completed results remain completed.

Both normal restart and final exit request explicit helper Release acknowledgements
after backend cleanup. Final exit has a 120-second limit and returns a failure if
cleanup or radio restoration cannot be confirmed; the user service allows 150 seconds
before the service manager's hard limit. Partial cleanup failure does not skip the
other services. This does not prove hardware restoration on an absent real radio,
or close every startup-error resource path inside protocol libraries.

Passed: 12 daemon tests, including delayed startup/cleanup, a bounded-queue event
backlog, partial cleanup failure and actual Unix-socket release acknowledgements;
daemon all-target Clippy; actual SIGTERM/SIGINT with an accepted stalled TLS upload
(partial removal, terminal history, immediate same-port restart); existing private
D-Bus/HTTPS consent/restart/idle tests; real portal/descriptor plus active-download
idle-exit test. The staged systemd user unit validates. The signal integration is
now part of CI. No running demo or installed package was changed.


## LocalSend timestamps, 2026-10-07

Source descriptors now supply optional RFC3339 `metadata.modified` and
`metadata.accessed` fields before reading payload bytes. Upload receive and the
explicit download client apply valid times only after size/checksum validation,
before no-replace publication. The storage layer flushes buffered writes and sets
times on a cloned descriptor of the private pending inode; remote names never
become metadata-update paths. Missing/null fields and unknown optional fields
remain compatible. Invalid date strings and non-representable leap seconds are
ignored independently, leaving otherwise valid contents and times usable.

Normal LocalSend upload metadata follows the primary v2.2 specification:
https://github.com/localsend/protocol/blob/main/README.md#41-preparation-metadata-only
Download offers also include these optional fields; clients that understand them
can retain the same timestamps, while clients ignoring them keep their existing
behavior. Filesystem timestamp range/precision still applies.

Passed: all 17 LocalSend and three storage tests, including real pinned-TLS
send/receive with empty files, collisions and a renamed/replaced source path;
reverse-offer timestamp preservation; UTC offsets, nanoseconds, a pre-epoch date,
invalid/null/unknown metadata; symlink collision without changing the original
file's timestamp. All-target LocalSend/storage Clippy passed. A separate actual
D-Bus/HTTPS daemon test sends explicitly written metadata JSON (including future
fields) and checks exact saved nanosecond timestamps before reading the contents.
This is software acceptance, not official mobile-client or physical-device proof.
The installed demo/packages remain unchanged pending final builds.


## External Bluetooth advertisement release, 2026-10-07

Implemented the BlueZ `LEAdvertisement1.Release()` callback with verification of
the registering daemon's unique D-Bus owner. Explicit cleanup is tied to that same
owner and never targets a replacement bluetoothd process. A released registration
is observed by its own handle rather than inferred only from controller-wide
instance counts. Quick Share's receiver renews its advertisement; its sender exits
with an actionable component failure. AirDrop keeps its functional AWDL receiver
and publishes a Bluetooth-wake recovery message, translated in GTK. Registration
rechecks controller power and never powers on or substitutes another controller.

The private BlueZ tests passed release authentication, another advertisement taking
the slot, no redundant unregister, and powered-off registration refusal. A new test
runs the actual Quick Share receiver/sender advertisement actors against the same
mock: receiver re-registration, sender failure, selected hci1 while hci0 is also
powered, and acknowledged cancellation cleanup. The existing CI runner now runs
both private-bus tests. These results do not close the broad controller rows:
all scanner/GATT/L2CAP paths, hardware power-loss, Bluetooth daemon restarts and
physical simultaneous-role acceptance still need their corresponding evidence.

Verification for this follow-up also passed all-target workspace Clippy and the
nine AirDrop/Quick Share library tests, including TLS/UKEY2 consent and exact-byte
loopbacks. No physical radio or installed-demo state was changed.


## Advertisement controller/daemon loss, 2026-10-07

Advertisement handles now monitor authenticated owner-specific PropertiesChanged,
InterfacesRemoved and the bus daemon's NameOwnerChanged signals. Power loss,
controller removal and replacement of org.bluez end that registration, even if
no Release callback arrives. An owner/power snapshot after registration closes
the signal-subscription race. Cleanup still targets the original unique owner;
the newly started service cannot receive stale UnregisterAdvertisement requests.
BlueR dispatches signals to all matching subscribers so its ordinary adapter
routing does not consume the advertisement lifecycle notification first.

The guarded private-bus tests now exercise forged power signals (ignored), actual
power changes, controller removal without Release, and replacement of the BlueZ
owner while the old process remains reachable and the replacement has an unrelated
advertisement. The actual Quick Share receiver actor re-registers against a
replacement service on its selected controller and keeps running. AirDrop's
message accurately describes Bluetooth wake loss while retaining AWDL reception.
Both private-bus suites and all-target workspace Clippy passed. These tests prove
advertisement lifecycle behavior, not complete scanner/GATT/L2CAP recovery or
physical hardware interoperability; those broader rows remain open. Demo and
installed packages are unchanged.


## Bluetooth server task ownership, 2026-10-07

GATT and L2CAP now retain all connection/notify, inbound, and advertisement-cycle
work in bounded per-server task groups. Ending a BLE bridge preserves an inbound
session that has migrated to Wi-Fi; backend shutdown closes admission, cancels
all owned work, and waits for resources to be dropped. Late callbacks cannot
reopen a drained group. GATT acknowledges StartNotify immediately, limits queued
writes and message assembly, and rejects payloads for other service hashes.

GATT application cleanup also waits for BlueZ acknowledgement and local exported
object removal. Its registration worker survives cancellation while a method
reply is outstanding and cleans uncertain failed registrations; cleanup remains
bound to the original unique daemon owner. The private-bus test exercises the
real GATT callbacks, queue exhaustion, oversized writes, immediate notification
setup, held unregister acknowledgements, removed objects, abandoned registration,
failed registration and successful scanning after shutdown. Two additional task
ownership tests cover migrated-session lifetime and cancellation of stalled work.
Workspace all-target Clippy passed. Real Bluetooth transport/Wi-Fi migration and
controller loss recovery across all roles remain separate acceptance/implementation
items; these tests do not mark those broader rows complete.


## IPv6 LAN and WIFI_LAN candidate negotiation, 2026-10-07

Quick Share listens and advertises on enabled bound IPv4 and IPv6 addresses.
Discovery probes both families; link-local records receive an explicit enabled
interface scope, retained through the endpoint and send API. Unscoped link-local
connections and scopes outside the allowlist are rejected, and local addresses
are not rediscovered as peers. Readiness no longer treats IPv6-only LAN as missing.

WIFI_LAN upgrade offers are built from the listeners actually opened for the
transfer. The wire schema now includes ServiceAddress and the ordered
address_candidates field: IPv6 precedes IPv4, legacy fields match the final
candidate, and a received candidate list supersedes those legacy fields. Invalid
lengths, ports, mapped addresses, loopback, multicast and link-local upgrade
addresses are rejected. Connection attempts remain ordered and bounded. This
implements the LAN portion of the
[Google wire schema](https://github.com/google/nearby/blob/main/connections/implementation/proto/offline_wire_formats.proto);
Wi-Fi Direct ip_v6_address and hotspot candidates remain separate active work.

Checks passed: 11 Quick Share tests, including real UKEY2/consent/exact-byte
transfer over IPv4 and IPv6, candidate rules and scoped discovery; the existing
private network-namespace engine lifecycle test now removes all non-loopback
IPv4 addresses and proves IPv6 readiness, connection, advertised address and
candidate connection before link-down/recovery. The address-policy/socket test
also passed as unprivileged Ubuntu. Workspace all-target Clippy passed. No
physical Android or over-the-air multicast acceptance is inferred, and installed
packages/live demo remain unchanged.


## Direct/hotspot IPv6 transport, 2026-10-07

Wi-Fi Direct's IPv6 link-local credential and hotspot ordered ServiceAddress
candidates now feed a shared dedicated-interface connector. Hotspot lists replace
legacy gateway/port fields; direct IPv6 precedes the IPv4 gateway. Every attempt
binds both the local address and leased device, scopes link-local peers locally,
rejects foreign scopes, and remains bounded/cancellable. Hosted sockets likewise
bind the reserved interface and advertise their actual IPv6/IPv4 listeners.
The hotspot legacy gateway stays the final IPv4 candidate.

Temporary NetworkManager profiles enable IPv6 without default routes, imported
DNS or imported routes. Joining supports IPv6-only activation, with an explicit
link-local mode when the offered IPv6 candidates require no router advertisement;
hosting retains an IPv4 gateway plus IPv6 link-local. IPv4 presence is now optional
in a join guard rather than represented as a fabricated address. Profile policy
follows the [NetworkManager IPv6 settings](https://networkmanager.dev/docs/api/latest/settings-ipv6.html).

Passed: candidate validation/fallback tests, the private NetworkManager test
(including IPv6-only and link-local profile cleanup), four network crate tests,
and the isolated kernel network test establishing scoped link-local connections
on the selected interface and refusing a different interface. Workspace all-target
Clippy passed. Native P2P group-owner role negotiation and netd's IPv4-required
DHCP completion path still need work for full IPv6-only supplicant P2P; no physical
radio interoperability or complete Wi-Fi Direct conformance is claimed here.


## IPv6-only supplicant group readiness, 2026-10-07

Netd now returns real optional IPv4/IPv6 addresses for an owned P2P group instead
of requiring an IPv4 DHCP result. Readiness inspects the exact ownership-marked
kernel interface and rejects down, tentative, duplicate-address-failed and expired
addresses. IPv6 readiness requires a usable link-local address. DHCP acquisition
and renewal may continue while the lease is owned, but a DHCP child exit does not
revoke a group that retains usable IPv6. Cancellation and group restoration retain
the existing lease boundary; no host DNS or default routes are configured.

Passed: six netd unit tests and `crates/linuxdrop-netd/tests/run-p2p-addresses.sh`
with its built binary. The latter uses a verified private kernel network namespace,
creates a marked group-like interface, proves immediate IPv6-only readiness without
a DHCP server, and rejects changed ownership. Workspace all-target Clippy passed.
The test does not emulate radio negotiation: complete P2P group-owner roles and
physical device interoperability still require their separate work/acceptance.


## Failed and cancelled startup ownership, 2026-10-07

The daemon now publishes radio ownership before fallible AirDrop setup and awaits
an explicit helper Release reply when either radio-backed backend fails to start.
Quick Share reservation validation also releases an invalid acquired lease before
falling back to LAN. Startup errors retain any restoration failure, and helper I/O
never holds the snapshot/data mutex. Generation-based quarantine remains intact.

Quick Share owns its engine and staging together from the first asynchronous
startup step. Aborting startup drains workers before deleting staging, including
when a full event queue prevents the initial ready event from being delivered.
Session cancellation belongs to the engine generation rather than whichever global
session token was most recently installed. mDNS advertiser/discovery objects own
their daemon before configuration can fail and await normal shutdown acknowledgement.

AirDrop owns mDNS from creation; setup failure and cancellation request shutdown,
with queue-full retry and a retained cleanup receipt. The listener is transferred
to the owning actor without an intervening await. Actor exit cancels connections;
listener, transfer, discovery and Bluetooth cleanup all run even if one fails.

Passed: 13 daemon tests (including delayed successful/failed helper release),
6 AirDrop tests (real discovery lifetime and consent/TLS/archive checks), all
12 default Quick Share tests, and workspace all-target Clippy. The targeted
Quick Share startup regression observes a real opened TCP port, aborts the caller,
and waits for the port and temporary staging directory to be released. The private
kernel LAN lifecycle test verifies discovery/listener recovery and normal stop.
These checks do not prove prolonged mDNS resource bounds or physical radio recovery.
The outer RQS cleanup-error propagation gap found in this pass is resolved by the
following change. Installed packages and the live demo remain unchanged.


## Quick Share shutdown result propagation, 2026-10-07

Top-level Quick Share workers now retain explicit cleanup failures and panics;
ordinary operational errors with successful cleanup remain recoverable. Discovery
shutdown, scanner StopDiscovery and GATT/advertisement removal failures carry a
typed cleanup marker through contextual errors. The engine waits for all workers
and returns a stable failed result rather than acknowledging an incomplete stop.
The adapter passes this through the daemon's persistent CommandSender receipt.
Failure events also reach the running backend instead of leaving a panicked worker
silently marked ready. Active discovery cannot be replaced without stopping it.

Multiple receiver advertisement removals begin together and all results are
collected, so one failure cannot skip the remaining handles. Failure results stay
available after a cancelled waiter and repeated shutdown requests. This does not
turn a failed cleanup into a successful retry without evidence of restoration.

Passed: 14 default Quick Share tests, including cancellation/panic/contextual-error
receipt regressions and real encrypted transfer checks; the private BlueZ suite
now drives the complete backend. It distinguishes a failed StartDiscovery with
successful cleanup (successful stop) from refused StopDiscovery (failed stop,
identical on retry). The private kernel LAN lifecycle test also checks duplicate
discovery rejection and normal acknowledged shutdown. Workspace all-target Clippy
passed. Physical Bluetooth recovery and prolonged resource-bound acceptance remain
separate; installed packages and the live demo were not changed.


## Autonomous Wi-Fi Direct group owner, 2026-10-07

The sender's WIFI_DIRECT host path now requests an actual temporary autonomous
P2P group through the leased-radio helper instead of labelling an ordinary NM
hotspot as Wi-Fi Direct. The helper requires P2P-GO capability and an allowed
non-DFS initiating channel; supplicant supplies the actual SSID, passphrase and
frequency. Credentials are not written to the lease journal and Debug omits them.
The group identity is journaled before network setup. A bounded DHCP server owns
only that group interface, selects a private /24 without existing-route overlap,
and supplies neither a default gateway nor DNS. Host readiness waits briefly for
IPv6 link-local DAD; listener binding still uses only the owned group interface.
A dead GO DHCP server revokes its lease rather than using the client-only IPv6
fallback. The Quick Share guard retains the dedicated-radio permit and requests
cleanup on cancellation or transfer completion.

Passed: seven netd and four network default unit tests; a private supplicant bus
contract verifies GroupAdd arguments, actual credentials, wrong-channel cleanup,
refusal to disconnect an existing interface and GroupStarted racing cancellation.
A separate verified kernel network namespace performs a real DHCP exchange over
veth, avoids an occupied subnet, observes usable IPv6 and verifies that the client
receives no router/DNS options. All-target Clippy passed for the app, network,
netd, daemon and Quick Share. These tests do not emulate over-the-air negotiation.
Full medium-role/device-name authentication negotiation, supplicant owner-loss
recovery and physical Android acceptance remain open; no complete Wi-Fi Direct
conformance or installed-package acceptance is claimed. Live demo unchanged.

Primary protocol references: [supplicant D-Bus GroupAdd/GroupStarted API](https://w1.fi/wpa_supplicant/devel/dbus.html)
and [Nearby offline wire formats](https://github.com/google/nearby/blob/main/connections/implementation/proto/offline_wire_formats.proto).


## Hardware-backed upgrade roles and receive-side group hosting, 2026-10-07

The helper's reserved-radio response now carries station/AP/P2P modes and enabled
frequencies. Missing fields in an old journal default to no capabilities. Hosting
requires permitted initiating channels; the current NM hotspot profile is limited
to its actual 2.4-GHz band. Quick Share no longer hardcodes 5-GHz or hosting support.
Its connection metadata and offered direct/hotspot media use the owned radio;
a missing P2P connector removes the P2P host/client operations. Both roles check
these capabilities again before a radio-changing operation.

An explicit peer client-role exclusion or incompatible authentication list is
honored. Missing legacy metadata remains compatible with password-based peers.
Unsupported requests fail instead of inventing a hotspot offer. Receive-side
hosting now selects an actual supplicant P2P group when both sides support it;
ordinary hotspots remain a separate medium. Both directions use one credential
generator, which rejects a medium that does not match the guard's owned network.
The role/auth fields were checked against [Google's wire schema](https://github.com/google/nearby/blob/main/connections/implementation/proto/offline_wire_formats.proto).

Passed: 13 daemon tests, 8 netd tests and 16 default Quick Share tests; workspace
all-target Clippy. New tests cover regulatory/mode exclusions, no-radio and
2.4-only metadata, peer role/authentication incompatibility, the actual serialized
TCP ConnectionRequest, exclusive helper ownership and cleanup, and shared direct
offer credentials. `run-direct-upgrade.sh` with the built rqs_lib unit-test binary
also passed in a verified private kernel network namespace: a simulated group
provider supplies an actual interface; an encrypted BLE-style duplex offer moves
to real TCP, exchanges introduction/ack and channel-drain frames, then validates
an encrypted keepalive with the original keys and sequence counters. Ordinary LAN
policy excludes the group interface, so only the dedicated group path can bind it.
The first fixture run lacked an up loopback interface; fixing that isolated
namespace setup made the targeted scenario pass in 0.23 seconds.

This is software negotiation/channel migration evidence, not over-the-air WPS,
physical Android acceptance or a complete direct-mode claim. Device-name-only
hosting and the receiver's dynamic client-role path remain active implementation
work. The live demo and installed packages remain unchanged.

2026-10-07 integrated runner: `python3 packaging/ci/network-lifecycle.py`
passed all four isolated kernel fixtures locally: IPv6-only P2P readiness,
DHCP without router/DNS options, live LAN address/link reconciliation, and
receiver-hosted Direct upgrade with encrypted sequence continuity. The runner
derives test executables from Cargo JSON and is wired into Linux CI; remote CI
has not run for these unpushed changes. No physical radio acceptance is implied.


## Joint radio allocation and deferred hotplug, 2026-10-07

The daemon now plans AirDrop and Direct Wi-Fi together before acquiring either
radio. It maximizes the number of radio-backed protocols, then gives a lone
AWDL-capable radio to AirDrop because Quick Share retains its LAN transport.
Preferences and deterministic capability scoring resolve remaining choices.
This avoids assigning the sole AWDL-capable adapter to Quick Share when a second
station-capable adapter is available. AirDrop starts first using the planned
radio/channel. After a failed AirDrop start, Quick Share can use a successfully
released radio; failed-cleanup ownership remains excluded by kernel phy.

Quick Share now selects suitable idle hardware automatically, including when no
preferred ID is set. Selection excludes rfkill, absent drivers, active/connecting
interfaces, leased PHYs, unavailable channels and incompatible modes. Automatic
USB opt-out is honored, with explicit preference remaining an intentional choice.
Changing that option now goes through the backend restart/admission guard. No
active injection test is implied by passive selection. The privileged helper
still validates live state before acquisition.

Hotplug considers both backends. A topology change during a transfer, active
link or restart is retained until idle rather than lost. Repeated inventory
polls do not repeatedly queue restarts, and temporary-interface churn after a
failed startup cannot cause unlimited automatic attempts for the same selected
attachment. Unplug/replug or rfkill resets that automatic attempt budget; the
explicit restart action remains available. The Hardware screen shows the backend
holding each radio reservation and disables active diagnostic controls on it.
A reservation can be held for pending cleanup; it is not a ready-state claim.

Validation: eight hardware tests (including joint scarce-capability assignment,
order independence, preferred-but-busy refusal, USB opt-out, ID/PHY exclusions,
regulatory constraints, postponed hotplug and failed-engine interface churn)
and thirteen daemon tests passed. App, hardware, netd and daemon all-target
Clippy with warnings denied passed. These are selection/lifecycle software
fixtures; physical radio unplug, monitor/injection and device interoperability
remain separately unverified. The user's live demo was not changed.


## Supplicant generation ownership and revocation, 2026-10-07

Both group hosting and WPS client discovery/connection resolve a unique
supplicant owner after subscribing to its NameOwnerChanged signal. Every root,
peer, group and cleanup call uses that unique destination. Group identities
persist the owner and D-Bus server GUID; matching object paths on a replacement
service or a different bus are insufficient authority. Setup, formation and
cleanup waits are bounded. Owner changes abort both directions, with cleanup
addressed only to the original connection. A still-connected old owner can
clean up its own group after relinquishing the well-known name; a dead owner
never redirects cleanup to the replacement.

The helper's watchdog reads the current service identity outside its global
state lock and revokes P2P leases on loss/replacement. Revoked or orphaned leases
are excluded from healthy Status responses, so the daemon's existing health
path retires peers/transfers instead of treating pending restoration as ready.
Retained records remain available through RecoveryStatus/RetryRecovery, and
channel changes/new group operations on revoked reservations are refused.
Already-revoked records do not repeatedly run restoration in the active watchdog.
Legacy journals deserialize, but records lacking service identity cannot send
ambiguous Disconnect calls; unresolved ownership needs explicit recovery rather
than assuming identical paths mean the same group.

Passed: the private supplicant contract now exercises two actual D-Bus owners
exporting identical paths, delayed GroupStarted and cancellation races, host
owner loss, client-discovery owner loss, old-owner cleanup, old-owner death,
legacy identity and wrong-bus rejection. No cancellation/discovery-stop or group
disconnect reaches the new owner. Its final native-bus run passed in 0.82 s.
Default tests passed: 4 network, 9 netd and 13 daemon; the netd regression checks
healthy Status versus retained recovery and refusal of channel mutations.
All-target Clippy for network/netd/daemon passed. Physical supplicant/radio crash
restoration is still a separate acceptance task; these fixtures do not certify
complete Wi-Fi Direct role/authentication interoperability.

The generation model follows the [D-Bus name/owner specification](https://dbus.freedesktop.org/doc/dbus-specification.html#message-bus-names)
and the [supplicant P2P D-Bus API](https://w1.fi/wpa_supplicant/devel/dbus.html).

## Receiver joins a sender-hosted upgrade, 2026-10-07

After file consent, a receiver that cannot host a suitable network can now send
UPGRADE_PATH_REQUEST and join the sender's offered direct group or hotspot. The
request intersects reserved-radio capabilities with explicitly advertised peer
host roles and compatible authentication. Password joining uses the existing
owned NetworkManager path; device-name/PBC joining uses the reserved P2P helper.
No request is sent without the receiver's actual advertised endpoint identity,
which is carried through GATT and all supported L2CAP entry paths.

While joining and connecting, encrypted BLE traffic continues through the payload
assembler and local cancellation remains responsive. Framing retains partial
headers and bodies across competing futures. Client introductions keep encryption
enabled; an advertised introduction ACK is required and validated. Legacy peers
that do not advertise ACK support do not incur an ACK read. Invalid offers or
failed setup send UPGRADE_FAILURE and release the owned network. An incomplete
prior-channel drain ends the session instead of claiming a successful switch.
Early LAST_WRITE arriving before the TCP ACK is retained for the final handoff.

The private kernel fixture now covers both receiver roles. Its new client case
uses real interface-bound TCP sockets and encrypted frames with a simulated P2P
helper. Scenarios cover consent gating, the exact advertised endpoint identity,
24 buffered payload chunks across setup, normal/no-ACK transitions, invalid ACK
and BLE recovery, local cancellation and cleanup, truncated drain, unrequested
media and invalid addresses. Separate tests cover interrupted frame reads and
role/authentication selection. All adapter-changing operations remain confined
to the fixture namespace; no physical radio or Android device is simulated as
verified hardware.

The implementation follows the event and role definitions in Google's
[wire schema](https://github.com/google/nearby/blob/main/connections/implementation/proto/offline_wire_formats.proto)
and ACK/role-switch behavior in its
[bandwidth-upgrade manager](https://github.com/google/nearby/blob/main/connections/implementation/bwu_manager.cc).
No additional upstream implementation code was copied. Device-name hosting,
remaining outbound handoff validation, full Bluetooth recovery/resource
coordination and physical interoperability remain separate completion work.

Validation for this pass: eleven protocol-library unit tests, sixteen Quick Share
adapter/integration tests (including the real UKEY2/file transfer), and both
private-kernel receiver-upgrade cases passed. The client case contains seven
success/failure scenarios. Protocol library/test Clippy and Quick Share all-target
Clippy passed with warnings denied; the unrelated upstream `tx_probe` example
still emits its existing dead-field warning when included in a combined
all-target invocation. Edited Rust files were formatted and diff checks passed.

Final review also covers consent overlapping an ordinary LAN handoff: acceptance
is retained and sent after the new transport is selected, while rejection keeps
its distinct terminal state and protocol response. Two focused regressions cover
these decisions; they are included in the unit count above.


## Device-name / WPS group hosting, 2026-10-07

Both sending and receiving host paths now negotiate password or device-name
Wi-Fi Direct authentication. Legacy peers retain password defaults; an explicit
unknown-only authentication list is refused. A device-name-only peer receives
the actual supplicant name, group frequency, bound IPv4/IPv6 endpoint and no SSID,
password or invented PIN. The helper request carries a typed authentication mode;
older requests default to password and unknown modes fail decoding.

On the exclusively leased group, LinuxDrop reads the parent's P2PDeviceConfig
and checks the new GO's WPS DeviceName matches. It never rewrites the shared
parent identity. Missing, invalid or mismatched names fail and clean up the new
group. WPS.Start (Role=registrar, Type=pbc) is sent only to the pinned unique
owner's newly created GO interface, after role/PHY/channel/credentials validation.
Group disconnect removes its WPS registrar together with the owned interface.
Socket EOF now cancels pending HostP2p as well as JoinP2p. File consent and the
existing encrypted Nearby session remain required; Wi-Fi identity is routing
metadata, not a trust decision.

The AP/GO dispatch is verified against the primary
[supplicant WPS handler](https://android.googlesource.com/platform/external/wpa_supplicant_8/+/refs/heads/main/wpa_supplicant/dbus/dbus_new_handlers_wps.c)
and [D-Bus API](https://w1.fi/wpa_supplicant/devel/dbus.html).
No upstream implementation code was copied. Device-name mode uses PBC; field 8's
future PIN extension does not require LinuxDrop to invent a PIN hosting variant.
Existing eight-digit PIN client offers remain accepted.

Validation: private D-Bus fixtures cover password without WPS, successful GO PBC,
WPS rejection, changed/empty device names, group cleanup, cancellation and service
owner replacement. Both private kernel channel migration cases pass; the hosting
case now runs password and device-name peers, checks the offered fields and
continues encrypted TCP sequence state. The client case retains its seven
success/failure scenarios. These checks use a simulated supplicant/helper and
real D-Bus/TCP/kernel isolation; they do not certify a physical WPS handshake.


## Checked bandwidth-upgrade handoffs, 2026-10-07

Outbound upgrade reads now share the persistent frame reader used by receivers;
a competing control/network future cannot discard a partially read header or
body. During radio joining, TCP connection, introduction acknowledgment and
prior-channel drain, local cancellation remains active and old-channel frames
continue through the encrypted dispatcher. The sender reads an introduction ACK
only when the offer advertises it, and checks version/type/event. The empty optional ACK submessage may be absent,
as in Google's own event-only constructor. Both
sender roles require LAST_WRITE and SAFE_TO_CLOSE before committing the swap.
EOF, invalid sequence/signature, timeout or rejection during that drain ends the
session; process_consent no longer turns a damaged handoff into BLE fallback.
Setup failures before LAST_WRITE retain BLE, report the failed medium, and
release any owned joined network. Early peer LAST_WRITE is retained; failure
after it is terminal.

Both receiver host paths and sender-hosted upgrades use the same bounded
introduction acceptor. It checks version, frame type, event, body and exact
four-byte endpoint before acknowledging. Failed, oversized or stray TCP clients
are closed while the established encrypted/control channel remains serviced.
Each introduction is limited to 1 KiB and five seconds, inside the existing
overall offer deadline. Sender identity comes from the fresh BLE target or the
mDNS service bytes carried into SendInfo, never from display names or IPs.
Receivers retain the endpoint from ConnectionRequest. This plaintext identity
is routing correlation, not authentication; encryption stays enabled and the
existing UKEY2 keys, sequence numbers and consent remain required.

Evidence: fourteen protocol unit tests, sixteen Quick Share wrapper/integration
tests (including UKEY2/exact file bytes), and both isolated kernel handoff tests
pass. The outbound regression has ten scenarios: ACK/no-ACK, event-only ACK, wrong type,
wrong version, oversized ACK, early LAST_WRITE with valid/invalid ACK,
truncated drain and local cancellation. Successful cases continue encrypted TCP
with matching sequence state. Both receiver host authentication modes reject a
closed client, wrong endpoint and oversized introduction before accepting the
correct client. Parser and discovery identity checks have focused regressions.
Physical peer acceptance remains separate; these fixtures do not prove a real
Android/Apple radio transfer. Full sender-hosted radio acceptance and broader
Bluetooth recovery remain in the overall completion audit.

Primary event/frame references: Google's
[bandwidth-upgrade manager](https://chromium.googlesource.com/external/github.com/google/nearby-connections/+/02354e29683393549b588a3ce2599a717f6da98e/connections/implementation/bwu_manager.cc)
and [frame constructors](https://chromium.googlesource.com/external/github.com/google/nearby-connections/+/8efa219dcfb6157fcc0791e45ef45989d681518b/connections/implementation/offline_frames.cc).
No upstream implementation code was copied.

Final source review confirmed Google's ForBwuIntroductionAck omits the optional
empty ACK body. Both client roles accept that event-only representation, and
both encrypted handoff fixtures exercise it. Requiring the body would reject a
valid Google peer; only introduction (not acknowledgment) needs its endpoint body.

## Recipient scan ownership, 2026-10-07

Both background recipient discovery and foreground BLE connection scans share
acknowledged ownership with FastInit scanning. Endpoint-ID selection avoids
same-name confusion; cache entries cannot perpetually refresh presence. Matching
transfer cancellation reaches pre-connect scanning and preserves its terminal
Cancelled state. Server exit drains owned scans, including listener-error exits.
Private BlueZ fixtures cover held method replies, original-owner cleanup,
recovery, shutdown and stable D-Bus client count. See the recipient-scan acceptance
record. Physical interoperability and prolonged signal/resource bounds remain
open; this does not close the overall Bluetooth/cross-protocol capacity row.
