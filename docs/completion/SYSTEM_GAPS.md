# System completion work

This audit follows the complete implementation plan, not a reduced 0.1 scope. Physical-radio acceptance is separate from missing software.

| Area | Existing evidence | Software completion |
|---|---|---|
| Radio inventory | sysfs identity, nl80211 modes/channels, NetworkManager protection | Firmware version, structured combinations/bands and explicit evidence profiles |
| Hardware diagnostics | Passive evidence only | Explicit authorized bounded monitor/raw-socket diagnostic with mandatory restoration; no automatic transmission or invented injection success |
| Recovery | Owned VIF journal and crash/disconnect cleanup | Structured recovery status and user-triggered retry |
| Quick Share direct radio | Idle-interface checks in protocol | Shared exclusive netd reservation, coordinated P2P negotiation/ownership |
| BLE receiver recovery | Selected-controller GATT/L2CAP/advertisement generations, acknowledged cleanup and retained migrated sessions; [private-bus evidence](../acceptance/BLUETOOTH_RECEIVER_RECOVERY_2026-10-07.md) | Sender advertisement recovery and shared single-slot turns now covered by [private-bus evidence](../acceptance/BLUETOOTH_SENDER_RECOVERY_2026-10-07.md). Outgoing recipient discovery and pre-connect scans remain; physical controller/L2CAP acceptance |
| BLE inventory | BlueZ power and advertisement counters | Remaining capacity and advertised feature details |
| File managers | Actual Nautilus 50, Thunar 4.20 and Dolphin 26.08 menu selection -> three-file ready draft; automatic Thunar Send To descriptor | New descriptor included in current DEB/RPM/Arch packages; actual Fedora actions and geometry pass. See [width/package follow-up](../acceptance/GTK_TRANSFER_WIDTH_2026-10-07.md) |
| Distribution | Tested Ubuntu DEB; Fedora 44/Arch x86_64 native source builds, private-root install/reinstall/removal, real daemon and installed GTK checks; Fedora and Arch revision upgrades | Booted service/polkit lifecycle; remaining Shell majors (46/50 now tested). See [native package evidence](../acceptance/PACKAGES_NATIVE_2026-10-07.md) |
| Flatpak | Installed GNOME 50 client: actual chooser -> document export -> daemon FD transfer; completed receipt Open With and folder-reveal portal actions verified, read-only export enforced | Default/per-request host folders and real HTTPS receives pass across grant revocation and portal/daemon restart. Notch preferences now use a fixed host route with failure/retry feedback; compatible host service required. See [folder evidence](../acceptance/FLATPAK_FOLDERS_2026-10-07.md) |
| CI | Rust/resource/package gates | Dependency/license audit, bounded parser fuzzing, tested Shell-version matrix |

Rules: never mutate the live workstation network; diagnostics run only on an explicit user action and a dedicated idle radio. No hardware whitelist converts upstream reports into local proof. Do not restart the live demo. Existing Windows storage is limited, so reuse build caches and bound isolated test environments.

Primary references: [kernel interface combinations](https://docs.kernel.org/driver-api/80211/cfg80211.html), [BlueZ advertising capacity](https://github.com/bluez/bluez/blob/master/doc/org.bluez.LEAdvertisingManager.rst), [Flatpak permissions](https://docs.flatpak.org/en/latest/sandbox-permissions.html).
