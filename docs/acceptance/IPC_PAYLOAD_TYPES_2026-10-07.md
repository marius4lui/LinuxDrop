# Shared settings and status types - 2026-10-07

linuxdrop-ipc now owns schema-v1 Settings and all 47 section fields. The daemon
resolves the user's download directory, then creates shared defaults. Type and
semantic validation are shared with GTK: integer bounds, enum-like choices,
protected active adapters, PIN rules and interface/controller syntax retain their
existing policy. Unknown additive response fields are tolerated and the GTK
client keeps the original JSON; unknown settings patches remain rejected by the
daemon's existing strict merge. Parsing is not permission to apply a new field.

The daemon builds Snapshot, PeerView, KnownPeer and TransferView values instead
of assembling their envelopes through ad-hoc JSON mutation. Core peer, transfer,
file and backend records are reused. GTK validates GetSnapshot, GetSettings and
GetDefaults before publishing them. Missing required data, wrong scalar types,
duplicate peer/transfer identifiers, unknown transfer directions and impossible
progress counters fail parsing/validation. Optional receive destination/selection
fields retain their existing wire layout. A malformed snapshot enters the
existing offline/retry path and disables remote actions.

Three payload regression cases check every settings leaf's wrong-type input,
wrong array elements, fractional/negative integer values, protected-adapter and
visibility constraints, missing required envelope fields, impossible file/total
progress and additive-field handling. Defaults must exactly match all serialized
typed fields. The two daemon settings policy/restart regressions also pass.
The typed Rust live-client probe now decodes snapshots, settings and defaults
through the shared validators. The release-daemon probe and existing D-Bus/HTTPS
consent, partial selection, preferences/history, download-offer and idle-shutdown
integration pass with these typed models.

The native GTK scenario passes in 20.20 seconds, including a wrong-type snapshot
sent over the real isolated D-Bus connection, disabled actions and recovery after
a valid response. Its visual fixtures now supply complete valid wire fields. The
numeric edit test previously assumed a fixed 400-ms window despite a deliberate
250-ms delayed write; it now waits up to two seconds for server and client receipt
state, preserving the exact byte-count assertion. The production numeric widget
was unchanged. App, daemon and optional IPC client all-target Clippy pass with
warnings denied.

This is not full JSON-contract completion: hardware inventory/helper annotations
remain a Value inside Snapshot, diagnostics/offer results and GNOME field-level
validation remain separate work. Existing wire signatures and protocol engines
are unchanged. No live-demo restart or browser use. Package evidence belongs in
`dist/BUILD_REPORT_0.1.0+review.20261007.25.json`.
