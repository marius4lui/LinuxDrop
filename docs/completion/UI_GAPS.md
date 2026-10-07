# Desktop completion audit

Binding references: `docs/AGENT_IMPLEMENTATION_PLAN.md` and the later explicit
panel-first interaction requirement. This is a working checklist, not a release
claim. Physical device acceptance remains separate from implemented software.

## Existing verified foundation

- [x] Native GTK4/libadwaita Send / Transfers / Hardware / Settings screens.
- [x] Shared multi-file draft, native chooser and real Wayland `Gdk.FileList` drop.
- [x] Explicit send and explicit incoming/code confirmation; no drop auto-send.
- [x] Top-panel button opens otherwise-hidden bubble; Quick Settings remains separate.
- [x] Native temporary GTK notch surface receives actual file payloads; outside/Escape dismissal.
- [x] Daemon owns system notifications; Shell does not duplicate them.
- [x] German/English baseline, explicit light/dark settings, small-window scroll/footer.
- [x] Loopback HTTPS accept/upload/saved-byte check and reverse-download revocation.

## Completion checklist

The focused review fixes and their native evidence are recorded in
[UI_COMPLETION_2026-10-06](../acceptance/UI_COMPLETION_2026-10-06.md). Unchecked
items retain their remaining acceptance scope even where code now exists.

- [x] Show individual invalid files and reasons, preserve valid selections, permit zero-byte files; asynchronous metadata validation.
- [x] Reconnect signal subscription on every proxy; owner-change invalidation, dirty snapshot retry and epoch/revision handling. GTK replacement is covered by an actual private-bus owner change; Shell by controlled replies in native GNOME.
- [ ] Stable peer/protocol selection across updates; favorites, custom labels, protocol preferences and soft-block controls using daemon contract.
- [x] Per-request destination and partial file acceptance using daemon contract.
- [ ] Full settings inventory from final schema: language/close behavior, receive policy/subfolders, public duration, protocol ports/modes, adapters/controllers, network filters, notifications/privacy/sound, limits/history, diagnosis/restart/reset/export.
- [x] Settings search survives immediate refreshes, includes desktop/diagnostic rows and translated option/category names; categories and advanced grouping remain available. No-results recovery resets both filters.
- [x] Desired/effective settings differences and backend apply errors visible: persistent applying/error/offline status in Settings, per-service details and recovery; native German 480x600 acceptance.
- [ ] Icon-only actions have translated accessible names; selected states/progress exposed; keyboard navigation/actions documented.
- [ ] Notch multiple-transfer selection remains stable; completion/error/open-folder actions; summary does not randomly change peer.
- [ ] Pointer-monitor option, position pinned during interaction, monitor removal recovery; configurable drag dwell and keyboard shortcut without conflict.
- [x] Adaptive wide-window navigation and targeted 480x600 and wide rendering.
- [ ] Fractional rendering acceptance.
- [x] Portal-origin file acceptance validated with a real document-portal export and descriptor handoff; revocation/closed-client lifetime and daemon access tested. Full installed Flatpak chooser acceptance remains in system packaging work.
- [ ] Assess folder/GVfs import safely; the original plan explicitly deferred these beyond regular local files, so any added import requires clear staging/cancel semantics rather than silent rejection.
- [ ] Redigierte Diagnose als Datei speichern; reset/restart dialogs explain active transfer impact.
- [x] Targeted native tests cover the reviewed draft, protocol, focus, settings and incoming decision regressions.

## Acceptance still requiring external environment or hardware

- [ ] Real screen-reader traversal in a complete desktop session.
- [ ] Physical mixed-DPI monitor hotplug and suspend/resume.
- [ ] Physical Apple/Android/USB/BLE interoperability (owned by protocol/hardware work).

The user's current demo session must not be restarted or modified for these
checks. Native tests use isolated runtime/config/data directories and disabled
or separately bound protocol listeners.


2026-10-07 follow-up: the acceptance record above now includes a second targeted
Astra UX/UI pass, failed-consent retry preservation, PIN dependency/search focus,
and a real isolated GNOME 46 bubble smoke at normal and German 150% text. Broad
unchecked rows remain open where their full acceptance scope exceeds those checks.

2026-10-07 focused polish: completed multi-file receive navigation, compact
filename review, launcher feedback and Shell keyboard transfer selection are
implemented and covered by native tests. See the acceptance record for exact
scope; broader accessibility and mixed-DPI acceptance remain open.


2026-10-07 additional Astra/high pass: per-device writes now have pending,
rollback and retry feedback; external device names are literal. Shell owner
replacement invalidates consent immediately, ignores stale completions and
coalesces in-flight updates; launcher failure remains visible. Native evidence
is in the acceptance record. The follow-up below also closes the GTK general snapshot owner-generation gap.


2026-10-07 GTK follow-up: stale proxy signals/replies cannot overwrite a new
owner's snapshot or enable consent; owner loss immediately disables remote actions
and closes old request dialogs. A fresh owner is queried independently of old
pending replies, while normal D-Bus activation remains available. Local file drafts
and completed-file actions survive. Native private-bus replacement/late-reply
regression and app Clippy passed. History values through 10000 display correctly.
The native runner now isolates configuration/data/cache and excludes LinuxDrop
activation from its private bus services; it cannot start the installed sharing
daemon during owner-loss tests.

2026-10-07 bounded two-agent follow-up: Astra/high agents independently owned
GTK settings and GNOME Shell; parent reviewed their implementation and native
renders. The Shell retains send/settings access during transfers, preserves
same-request keyboard focus across unrelated updates, isolates replacement
consent focus, treats external text literally, and aligns progress at the left
edge. Its English 1440x900 and German 150% 800x600 native checks passed. See
[Shell action acceptance](../acceptance/UI_SHELL_ACTIONS_2026-10-07.md).

The broad unchecked accessibility, monitor and settings-inventory rows above
are deliberately not treated as proved by these focused checks. This follow-up
is not an assertion that all LinuxDrop implementation work is complete.

The same follow-up adds persistent per-setting pending/error/retry feedback,
preserves failed text/reset drafts, rejects obsolete save receipts and late
snapshots, and serializes reset against field writes. The isolated native GTK
scenario and app all-target Clippy passed; parent inspected the German 480x600
failure/retry rendering. See
[GTK settings acceptance](../acceptance/UI_GTK_SETTINGS_2026-10-07.md) for exact
coverage and the bounded reset timeout during owner loss.

2026-10-07 further bounded Astra/high pass: the Shell holds remote actions
pending until a snapshot started after the action receipt completes, including
unchanged revisions; Quick Settings now exposes pending/error feedback. See
[asynchronous Shell acceptance](../acceptance/UI_SHELL_ASYNC_2026-10-07.md).
The parent also compared the current daemon settings defaults/validation with
all GTK categories. Every user-facing schema field has a control; schema_version
is internal metadata. Two numeric mismatches were corrected: zero-minute
visibility and the full byte-precise receiving limit. This source inventory does
not prove that every setting has passed an installed-session end-to-end test.

2026-10-07 final bounded Astra/high journey review: file removal preserves
keyboard focus, picker failures become visible, and incoming paths/received
filenames are literal from construction. The Shell clears an obsolete launch
error after an accepted retry. Existing isolated GTK and GNOME smoke scenarios
passed; see [GTK journey acceptance](../acceptance/UI_GTK_JOURNEYS_FINAL_2026-10-07.md)
and [Shell retry acceptance](../acceptance/UI_SHELL_RETRY_RECOVERY_2026-10-07.md).
The parent also inspected the compact German incoming-dialog render. These
scoped fixes do not close the broader installed-session or physical acceptance
items above. No live demo restart was performed.

2026-10-07 Shell 50 follow-up: the removed chrome option and Fedora private GI
library paths are corrected. Native 46/50 scenarios now include installed GTK
drop-window launch, measured centering after natural sizing, live spacing,
close/reopen and snapshots. See [Shell 50 acceptance](../acceptance/SHELL_50_2026-10-07.md).
The subsequent [GTK text scaling pass](../acceptance/GTK_FONT_SCALING_2026-10-07.md)
replaced fixed-pixel custom fonts and fixed the horizontal overflow exposed by
150% desktop text at compact width. Native send/settings/hardware/drop/incoming
renders and regression assertions pass. Actual external drag payload on 50,
screen-reader speech, fractional/multi-monitor acceptance and the remaining
Shell-version matrix are not asserted by these tests.

2026-10-07 native file-manager follow-up: Nautilus/Thunar/Dolphin actual menu
handoffs preserve a three-file selection with special characters. Installed
Fedora revision 3 GTK captures expose horizontal clipping at the default window
width; rebuild the current app there and diagnose any remaining 4.22/1.9 layout
incompatibility. See [file-manager acceptance](../acceptance/FILEMANAGERS_2026-10-07.md).
