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
- Per-session random inbound IDs (LAN, GATT, L2CAP); daemon transfer ID retained on outbound failures.
- Explicit SAS consent in both directions; hidden-mode offers are rejected by the adapter.
- Private per-session staging, exclusive file creation, name/count/size checks, exact end-of-file checks, receive publication through LinuxDrop ReceiveStore.
- Constant-time HMAC verification, bounded metadata buffers, invalid curve-point rejection and AES frame dimension checks.
- Persistent partial length-prefix state across control-channel wakeups; read timeout for complete frame bodies.
- Zero-byte send framing and changed/truncated source detection.
- Independent tracked LAN sessions, shutdown cancellation, inbound terminal failure events and discovery connect timeout.
- TCP/mDNS task health events, Bluetooth degradation diagnostics, initial mDNS registration and daemon shutdown on drop. Rust formatting is normalized for the repository's formatter gate.

AirDrop / AWDL:

- Filin checks every requested frequency against the netd-provided `LINUXDROP_ALLOWED_FREQUENCIES` regulatory allowlist; malformed present policy fails closed.
- LinuxDrop does not invoke Luftlift's auto-accept server or salvage archive decoder. It uses its plist builders/parser, mDNS metadata and self-signed server certificate generation behind a separate consent-aware server.
- LinuxDrop's AirDrop HTTPS client verifies the TLS signature and pins the receiver certificate across the whole transfer. This does not verify an Apple account or contacts identity.
- Strict bounded dvzip/CPIO processing, regular-file-only extraction, advertised filename matching, private spool files and no-replace publication.

The upstream library and CLI sources are retained for attribution and reproducible builds. Only LinuxDrop's adapter paths and the netd-launched Filin binary form the product runtime.
