# Full completion ledger

The user's full implementation mandate supersedes the earlier 0.1.0 scope cutoff.
Baseline passing tests and packages do not prove the changes in this pass. Mark
items complete only with current code and relevant evidence. Physical device
acceptance is separate; missing software is not a hardware limitation.

| Area | Baseline | Completion work and owner |
| --- | --- | --- |
| Native sending | GTK chooser/drop, multi-file draft, protocol selection | Desktop: async validation, submitted-file identities, stable protocol IDs, focus retention, small/wide layouts, accessibility |
| Panel bubble | Click to open; initially hidden | Desktop: bounded German actions, long names, multi-transfer selection, monitor behavior and preferences |
| Settings | Searchable persisted subset | Desktop + daemon: full schema, preserve dirty edits, actual policy application, reset/restart/export |
| Incoming consent | Accept/reject and SAS | Daemon + protocols + desktop: explicit destination/subset/collision, totals before acceptance, one decision across clients |
| LocalSend | Real discovery/TLS/transfer/reverse-offer server | Protocols: 2.2 receiver PIN, sender PIN challenge, partial acceptance, metadata, reverse client, network policy |
| Quick Share | Encrypted LAN, BLE and SSID upgrade | Protocols + systems: owned radio lease/profile, static port/controller policy, WPS/device-name upgrade, truthful status |
| AirDrop | Real TLS routes and helper AWDL engine | Protocols: receive options, controller policy, shaping, direction switches; physical interop remains experimental |
| Hardware | Passive inventory, protected active link, AWDL lease | Systems: firmware/bands/combinations/evidence, bounded explicit diagnostic, recovery UI, reserved direct-Wi-Fi lease |
| Daemon/IPC | JSON snapshot/revision and user service | Daemon: persisted peer preferences, history age, notification capabilities/privacy, idle close, typed shared contract, restart races |
| File safety | Held destination descriptor, private partials, atomic no-replace | Daemon + protocols: free-space preflight, policy-driven collision handling, retained outgoing file authority |
| Packaging | Tested Ubuntu DEB; native Fedora 44/Arch source builds and installed GTK/daemon checks, Fedora package revision upgrade | Systems: booted distro service/polkit lifecycle, Arch version upgrade, Flatpak host-client path, integration installation and license/dependency checks |
| Acceptance | Baseline recorded in docs/acceptance | All: targeted regressions, new release artifacts after integration; no blanket reuse of baseline result |

Focused source reviews: [UX](UX_REVIEW.md), [visual](VISUAL_REVIEW.md).
Detailed owner checklists: [desktop](UI_GAPS.md), protocol/system reports when
their audits are finalized. The two additional Astra/high reviewers inspect
bounded scopes and report once; implementation ownership stays with the desktop
agent to avoid conflicting edits and duplicate token use.

Unresolved external acceptance: real Android/Apple devices, actual radio injection
and BLE interoperability, physical adapter unplug and monitor hotplug. Simulations
and upstream evidence must not be reported as physical LinuxDrop acceptance.
