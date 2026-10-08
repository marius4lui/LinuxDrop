# Implementation and acceptance status

Updated 2026-10-06. Branch: `feature/linuxdrop-implementation`. **Full implementation is active, not complete.** The packaged 0.1.0 experimental baseline on Ubuntu 24.04 amd64 / GNOME 46 is a milestone, not acceptance of the full plan. The evidence below describes that baseline; current completion changes require their own validation. See [completion requirements](completion/REQUIREMENTS.md).

## Implemented

- Rust workspace, shared domain types, bounded command/event actors and versioned session D-Bus service.
- Native GTK4/libadwaita sending, file drop, transfer progress/decisions, hardware and searchable settings; German/English and light/dark themes.
- GNOME Quick Settings and a shell-positioned native notch drop surface. A top-panel icon explicitly opens the bubble; it is hidden until clicked.
- LocalSend discovery and real HTTP(S) send/receive, approval, rejection, cancellation, multiple/empty files, certificate pinning and optional expiring PIN download links.
- Quick Share/Nearby Share as one backend: LAN, UKEY2, code comparison at both ends, BlueZ and negotiated direct WLAN upgrades on an explicitly selected idle adapter.
- AirDrop TLS discovery/ask/upload, send/receive, strict bounded archives, optional BLE wake and AWDL through a separately authorized Filin helper.
- Passive udev/sysfs/nl80211/NetworkManager/BlueZ inventory, active-connection protection, regulatory channel selection, leases and recovery ownership.
- Private no-replace file publication, bounded persistent transfer history, visibility expiry/lock behavior, native notifications and opt-in auto-open/autostart.
- Ubuntu DEB, matching source archive, native services and file-manager actions. Fedora 44/Arch x86_64 native source builds and isolated installed-package checks now pass; see the native package acceptance record.
- Dedicated Ubuntu environment and a working live Windows/WSLg demo, with an explicit loopback-only HTTPS demo receiver. No fabricated devices are injected into the product.

## Verified evidence

| Check | Result |
|---|---|
| Workspace release build | Passed on Rust 1.99.0, Ubuntu 24.04 amd64 |
| Workspace formatting | Passed |
| Workspace Clippy, all targets, warnings denied | Passed; subsequent demo-only changes also checked |
| Workspace tests | 19 passed, zero failed |
| Filin regulatory-policy regression | Passed; forbidden/malformed frequency policies fail closed |
| Release daemon session integration | Passed: actual D-Bus, HTTPS approval and bytes, hidden mode, terminal state, settings validation, private persisted history across restart, clear-history preserving files |
| LocalSend | Real TLS both directions, multiple/zero-byte/colliding files, rejection, wrong certificate, traversal/size/replay cases and malicious receiver responses |
| Quick Share | Real TCP UKEY2/paired-key flow, equal SAS codes, consent at both ends, exact payload; unavailable BlueZ preserves LAN and occupied listener fails honestly |
| AirDrop | Real TLS Discover/Ask/Upload with consent, multi/empty files, collisions, bounded archive checks and cancellation during pending approval |
| Native UI | Actual Nautilus Wayland multi-file notch drag, cancelled drag, real GTK incoming approval with exact saved bytes, link revocation, light/dark/small-window captures |
| Package | Ubuntu initial install, same-version upgrade, remove and reinstall; helper/service/resource checks |
| Live Windows output | Native app and full GNOME desktop actually rendered on the host; black nested rendering traced to XShm UID mismatch and corrected using the existing UID-1000 Ubuntu user |

Details: [native UI](acceptance/UI_NATIVE_2026-10-06.md), [packaging](acceptance/HARDWARE_PACKAGING_2026-10-06.md), [WSLg rendering](acceptance/WSLG_RENDERING.md), [protocol design](adr/0002-protocol-engines.md). Local checks are recorded here; no unrun GitHub Actions workflow is claimed green.

## Remaining device/desktop acceptance

No physical Android, iPhone, macOS, Bluetooth or Wi-Fi adapter was available in this guest. Actual AWDL injection/channel behavior, USB unplug/recovery and Quick Share direct-Wi-Fi/BLE combinations need a suitable device matrix. Upstream compatibility claims are not LinuxDrop test results. AirDrop remains experimental and initially disabled.

Also not yet validated: GNOME versions other than 46, fractional scaling and monitor hotplug, sandbox file-portal drops, a complete screen-reader traversal, booted RPM/Arch service lifecycles, exhaustive fuzzing and an independent security audit. The verified Wayland drop source was native Nautilus. These limits do not prevent the tested Ubuntu package or live local demo from running.

## Completion scope

The full agreed settings inventory and integrations remain in scope. The current completion pass implements per-peer preferences, bandwidth/interface policy, per-transfer destinations and selections, LocalSend PIN handling and the remaining native UX and hardware controls. A control is only complete when its backend behavior and meaningful acceptance checks exist. Apple Contacts Only cannot be promised without the required Apple identity mechanism. Quick Share direct networking is a negotiated transport upgrade; its missing software paths remain work items.

All production backends use real protocol paths. The separate opt-in demo receiver automatically accepts only on loopback and is never installed as a production service. The main application always retains its consent flow.
