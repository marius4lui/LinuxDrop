# GNOME status validation - 2026-10-07

The Shell now validates every GetSnapshot response before publishing state or
settling actions. Structural types come from the actual Rust Snapshot, Settings,
Peer and Transfer models via optional Schemars derives. Number limits and setting
choices are shared with Rust validation rather than copied into the schema.
`tools/sync-ipc-schema.py --check` detects stale JSON/GJS exports in CI.
The generation feature is not enabled in normal daemon or app builds.

The small GJS reader supports only the exported schema vocabulary and rejects new
assertion keywords at module initialization. References are local definitions;
no network or external schema resolution occurs. Cross-field validation covers
unique IDs, progress/size consistency, device-name and path bounds, distinct
ports, PIN requirements and adapter/interface syntax. Unknown additive fields
remain opaque and do not grant permissions. JavaScript integers must be safe;
unsafe values are rejected rather than used as rounded revisions or counters.

The parser bounds input to 32 * 1024 * 1024 UTF-16 code units, one million validation
visits and depth 64. Oversized or invalid status clears stale transfer data and
disables remote actions through the existing offline/retry path. A later valid
snapshot restores the controls. An unrelated pending mutation keeps its existing
receipt/refresh ownership. Hardware detail remains an opaque object in this
schema; detailed hardware, diagnostic and offer response models remain open.

Verification:

- Pure GJS regression: 128 assertions covering all settings leaf types and
  required fields, malformed status, bounds, duplicate IDs, impossible progress,
  nullable selection, additive data and unsupported assertion keywords.
- Actual private-daemon D-Bus/HTTPS integration: 36 observed status responses
  accepted by the GJS reader across consent, receive/download, PIN and completion.
  This integration is enabled in CI with LINUXDROP_TEST_GJS_STATUS=1.
- Native GNOME 46 / Mutter 46.2 Shell and preferences smoke passed as unprivileged
  linuxdrop at German 150 percent text on 800x600. Added malformed-response and
  valid-recovery assertions verify absent consent and disabled Quick Settings.
  The first run exposed test setup reading defaults after clearing its snapshot;
  moving that capture before the explicit offline transition fixed the fixture.
- IPC all-target Clippy with schema/client features, generated-schema freshness
  and git diff --check passed. Existing typed payload regressions were already
  green for the same Rust model/limits in the preceding turn.

No visual redesign, browser use, live demo restart or physical radio acceptance
is claimed. The test service uses isolated configuration and loopback transfers.
The normal release binaries match revision 26 because runtime Rust sources have
not changed since that release; revision 27 packages the validated Shell reader.
