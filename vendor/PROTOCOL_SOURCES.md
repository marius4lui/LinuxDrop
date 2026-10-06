# Pinned protocol sources

LinuxDrop incorporates the following source snapshots, retaining their LICENSE files. These are source dependencies compiled into native binaries; no runtime Git download or remote script execution is used.

| Source | Commit | Vendored subset | License |
|---|---|---|---|
| [open-quickshare](https://github.com/ignotusbucius/open-quickshare) | `5a31145163ee22ab9cf1c7d3dffd74355febe93f` | `core_lib` | GNU GPL version 3; upstream LICENSE retained |
| [opendrop-rs](https://github.com/ayourtch-llm/opendrop-rs) | `dccc798e244363eb92d35e3c52e9a913188dda91` | `filin-rs`, `luftlift-rs` and workspace manifest | GPL-3.0-only, declared by upstream workspace |

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
