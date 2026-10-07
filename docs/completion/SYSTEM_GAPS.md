# System completion work

This audit follows the complete implementation plan, not a reduced 0.1 scope. Physical-radio acceptance is separate from missing software.

| Area | Existing evidence | Software completion |
|---|---|---|
| Radio inventory | sysfs identity, nl80211 modes/channels, NetworkManager protection | Firmware version, structured combinations/bands and explicit evidence profiles |
| Hardware diagnostics | Passive evidence only | Explicit authorized bounded monitor/raw-socket diagnostic with mandatory restoration; no automatic transmission or invented injection success |
| Recovery | Owned VIF journal and crash/disconnect cleanup | Structured recovery status and user-triggered retry |
| Quick Share direct radio | Idle-interface checks in protocol | Shared exclusive netd reservation, coordinated P2P negotiation/ownership |
| BLE inventory | BlueZ power and advertisement counters | Remaining capacity and advertised feature details |
| File managers | Nautilus/Dolphin action source, Thunar example | Safe idempotent Thunar installer preserving existing actions; native execution checks |
| Distribution | Tested Ubuntu DEB; Fedora 44/Arch x86_64 native source builds, private-root install/reinstall/removal, real daemon and installed GTK checks; Fedora revision upgrade | Booted service/polkit lifecycle and Arch cross-version upgrade; newer Shell matrix. See [native package evidence](../acceptance/PACKAGES_NATIVE_2026-10-07.md) |
| Flatpak | Not present | Optional native GUI using the separately installed host daemon, restricted named D-Bus access and file portal compatibility |
| CI | Rust/resource/package gates | Dependency/license audit, bounded parser fuzzing, tested Shell-version matrix |

Rules: never mutate the live workstation network; diagnostics run only on an explicit user action and a dedicated idle radio. No hardware whitelist converts upstream reports into local proof. Do not restart the live demo. Existing Windows storage is limited, so reuse build caches and bound isolated test environments.

Primary references: [kernel interface combinations](https://docs.kernel.org/driver-api/80211/cfg80211.html), [BlueZ advertising capacity](https://github.com/bluez/bluez/blob/master/doc/org.bluez.LEAdvertisingManager.rst), [Flatpak permissions](https://docs.flatpak.org/en/latest/sandbox-permissions.html).
