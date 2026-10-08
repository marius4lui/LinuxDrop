# GTK download-link lifecycle - 2026-10-07

This bounded pass first reviewed the existing selection, file-journey and numeric
settings acceptance. No further main/settings/hardware restyle was justified.
It corrected a concrete gap in the download-link flow:

- Preparing a link now shows the existing translated `Preparing…` label and
  disables both creation and sending until completion. Repeated activation
  cannot start overlapping offers. Failure preserves the file selection and
  restores the action for an intentional retry.
- Confirmation and active-link dialogs follow the sharing service lifecycle.
  A replacement service cannot receive an old dialog's create/stop action, and
  a late old-service response cannot reopen a stale link dialog.
- Closing the active dialog after a same-service snapshot failure still revokes
  its offer. If creation succeeds while that same service is temporarily offline
  in the UI, the unseen offer is revoked before allowing another creation and
  its state is refreshed. Failed cleanup remains recoverable through the existing
  active-link status/stop action when snapshots return.

The existing private D-Bus/Xvfb native scenario was extended with held offer
responses, duplicate activation, failure/retry and preserved files, successful
link display, same-service close/revocation, and a delayed response across real
bus-owner replacement. Its existing selection, settings and incoming-transfer
assertions also passed. The native scenario ran as the `linuxdrop` user with
isolated runtime/config/data/cache and no installed LinuxDrop activation.

Validation: `cargo test -p linuxdrop --no-run`, app all-target Clippy with
`-D warnings`, the isolated native scenario (1/1, 22.32 seconds), and
`git diff --check` passed. Portal/PipeWire and private-bus shutdown messages were
environmental. The [German compact pending capture](ui/link-lifecycle/download-link-preparing.png)
was inspected: the closing confirmation remains readable above the disabled
actions and `Vorbereiten …` feedback. The lifecycle assertions are the primary
evidence; this capture is not an end-to-end live transfer claim.

The successful-creation-while-offline cleanup branch was compiled and reviewed;
it was not separately induced by the native scenario. Active-link cleanup and
real owner replacement were exercised.

Real device transfer, installed-session portal behavior, screen-reader speech
and physical hardware remain outside this pass. No browser was used, no live
demo was restarted, and no changes were committed.
