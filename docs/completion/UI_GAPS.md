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
- [ ] Reconnect signal subscription on every proxy; owner-change invalidation, dirty snapshot retry and epoch/revision handling.
- [ ] Stable peer/protocol selection across updates; favorites, custom labels, protocol preferences and soft-block controls using daemon contract.
- [x] Per-request destination and partial file acceptance using daemon contract.
- [ ] Full settings inventory from final schema: language/close behavior, receive policy/subfolders, public duration, protocol ports/modes, adapters/controllers, network filters, notifications/privacy/sound, limits/history, diagnosis/restart/reset/export.
- [ ] Settings search survives edits and includes desktop/diagnostic rows; clear categories and advanced grouping.
- [ ] Desired/effective settings differences and backend apply errors visible.
- [ ] Icon-only actions have translated accessible names; selected states/progress exposed; keyboard navigation/actions documented.
- [ ] Notch multiple-transfer selection remains stable; completion/error/open-folder actions; summary does not randomly change peer.
- [ ] Pointer-monitor option, position pinned during interaction, monitor removal recovery; configurable drag dwell and keyboard shortcut without conflict.
- [x] Adaptive wide-window navigation and targeted 480x600 and wide rendering.
- [ ] Fractional rendering acceptance.
- [ ] Portal-origin file acceptance validated with a real document-portal export; document scope/lifetime and daemon access.
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
