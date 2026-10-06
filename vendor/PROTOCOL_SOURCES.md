# Pinned protocol sources

LinuxDrop incorporates the following source snapshots, retaining their LICENSE files. These are source dependencies compiled into native binaries; no runtime Git download or remote script execution is used.

| Source | Commit | Vendored subset | License |
|---|---|---|---|
| [open-quickshare](https://github.com/ignotusbucius/open-quickshare) | `5a31145163ee22ab9cf1c7d3dffd74355febe93f` | `core_lib` | GNU GPL version 3; upstream LICENSE retained |
| [opendrop-rs](https://github.com/ayourtch-llm/opendrop-rs) | `dccc798e244363eb92d35e3c52e9a913188dda91` | `filin-rs`, `luftlift-rs` and workspace manifest | GPL-3.0-only, declared by upstream workspace |
| [BlueR](https://github.com/bluez/bluer) | crate 0.17.4, upstream `8072d9cc6d034f1bfc464dda338d19a14f9badaf` | Published crate sources | BSD-2-Clause; upstream LICENSE retained |

Quick Share's remaining upstream Git dependency is pinned in its manifest: `Martichou/mdns-sd` at `c3d6ec2e173ac2cf8306943f70026c37c2ab1dd7`. The upstream AGPL-licensed `sys_metrics` dependency was removed: its only two calls obtained the hostname, now implemented independently using the Linux kernel's hostname file and a constant fallback. Cargo.lock pins registry packages. Do not replace pins with branch references.

## LinuxDrop modifications

Quick Share:

- Runtime Bluetooth setting, receive size/count policy, no unsolicited controller power-on.
- Dedicated explicitly selected disconnected interface for Wi-Fi upgrades; no first-radio choice or active connection disruption. Temporary connection names use the LinuxDrop namespace. Errors omit argument lists containing network passwords.
- NetworkManager upgrades now use lease-scoped volatile D-Bus profiles bound to the creating client, plus an exclusive transfer semaphore and ownership-checked asynchronous cleanup. The old fixed profile names and `nmcli` password arguments are removed.
- Added Google wire fields `ip_v6_address=6`, `pin=8`, `device_name=9`; device-name offers route through LinuxDrop's leased supplicant P2P connector. IPv6 credential routing and complete role negotiation remain explicit completion work; a decoded field is not advertised as working transport support.
- Per-session random inbound IDs (LAN, GATT, L2CAP); daemon transfer ID retained on outbound failures.
- Explicit SAS consent in both directions; hidden-mode offers are rejected by the adapter.
- Private per-session staging, exclusive file creation, name/count/size checks, exact end-of-file checks, receive publication through LinuxDrop ReceiveStore.
- Constant-time HMAC verification, bounded metadata buffers, invalid curve-point rejection and AES frame dimension checks.
- Persistent partial length-prefix state across control-channel wakeups; read timeout for complete frame bodies.
- Zero-byte send framing and changed/truncated source detection.
- LinuxDrop sends opened source descriptors into the outbound engine; logical names remain stable and path replacement cannot change which inode is read. Legacy upstream path inputs remain separate.
- Independent tracked LAN sessions, shutdown cancellation, inbound terminal failure events and discovery connect timeout.
- TCP/mDNS task health events, Bluetooth degradation diagnostics, initial mDNS registration and daemon shutdown on drop. Rust formatting is normalized for the repository's formatter gate.
- IPv4 LAN listeners, source sockets, discovery probes and upgrades follow LinuxDrop's interface allowlist. mDNS uses explicit bound addresses instead of the library's unrestricted auto-address population. Listener reconciliation publishes one snapshot for advertisement, discovery and readiness; existing sessions survive unrelated address changes. Discovery probes are bounded, cancellable and discarded when superseded.
- Shared signed-integer P-256 coordinate decoder restores leading zeroes for SEC1 and rejects negative/oversized coordinates, wrong key types and invalid points; both handshake directions use it.
- File payloads consume the daemon-provided shared bandwidth budget in both directions; matching cancellation remains responsive during a budget wait. The outbound BLE connector uses the explicitly selected controller too.

AirDrop / AWDL:

- Filin checks every requested frequency against the netd-provided `LINUXDROP_ALLOWED_FREQUENCIES` regulatory allowlist; malformed present policy fails closed.
- LinuxDrop does not invoke Luftlift's auto-accept server or salvage archive decoder. It uses its plist builders/parser, mDNS metadata and self-signed server certificate generation behind a separate consent-aware server.
- LinuxDrop's AirDrop HTTPS client verifies the TLS signature and pins the receiver certificate across the whole transfer. This does not verify an Apple account or contacts identity.
- Strict bounded dvzip/CPIO processing, regular-file-only extraction, advertised filename matching, private spool files and no-replace publication.
- Upload streams share the daemon payload budget; outbound IPv6 sockets bind the source address/device of the scoped leased AWDL interface instead of relying on the default route.

The upstream library and CLI sources are retained for attribution and reproducible builds. Only LinuxDrop's adapter paths and the netd-launched Filin binary form the product runtime.

## BlueR lifecycle patch

`vendor/bluer` contains the crates.io 0.17.4 source snapshot identified by its
original `.cargo_vcs_info.json`. The workspace patches that exact package locally.
Only `src/adv.rs` and `src/session.rs` differ from the published source (including
Rust formatting). The normalized upstream Cargo.toml and original Cargo.toml.orig
are retained. Registry cache markers and its package lockfile are omitted.

- `AdvertisementHandle::unregister()` waits for the BlueZ method reply and local
  object removal. The result persists; cancelling the wait does not cancel cleanup.
- An owned worker retains in-flight registration until its reply, including when
  the caller was cancelled. Failed registration is cleaned before reporting its
  error. Advertising calls have a 15-second D-Bus deadline; transient unregister
  errors retry while the daemon's separate cleanup receipt can time out honestly.
- Session teardown aborts its event/method dispatch tasks as well as the I/O
  driver, so stale connection clones do not retain a D-Bus owner.
- LinuxDrop uses a dedicated session per advertisement, isolating owner-loss
  cleanup from GATT and other protocols. ActiveInstances capacity checks remain
  serialized and never evict existing advertisements.
- Quick Share and AirDrop explicitly await unregister. Quick Share's former
  1.5-second replacement sleep and outer registration timeout are removed.

The private BlueZ mock test covers selected controllers, actual exported Apple
manufacturer/Quick Share service properties, slot exhaustion, held unregister
replies, retry, registration cancellation, drop cleanup and bus-owner removal.
Physical Bluetooth packet/controller acceptance remains separate.

Reference: [BlueZ LEAdvertisingManager1](https://bluez.readthedocs.io/en/latest/advertising-api/).
