# Focused UI/UX completion pass

Two additional Astra/high agents performed bounded, read-only UX and visual
reviews. Their reports are in `docs/completion/UX_REVIEW.md` and `VISUAL_REVIEW.md`.
One desktop implementation owner applied the findings; root integrated the daemon
contracts and completed the last incoming-selection fix after agent credits ran
out. The user's existing visible demo was not restarted.

## Implemented changes

- Compact file drop area and bounded file preview keep nearby devices reachable.
- Wide windows use header navigation; narrow windows retain bottom navigation.
  The send footer follows the content width.
- File selection survives asynchronous submission; additions made while a send
  starts remain in the draft. Zero-byte files are valid; individual invalid files
  remain visible with their reason.
- Protocol selection uses stable IDs. Peer widgets and keyboard focus survive
  unrelated transfer progress. Dirty settings entries survive reconstruction.
- Incoming review shows total size, code comparison where required, exact default
  destination including subfolders, per-file selection and collision policy.
  LocalSend accepts the selected subset natively. Quick Share/AirDrop explain that
  the complete bundle is transferred and only selected files are published.
- GNOME bubble confirmation has a full-width primary action; secondary actions
  remain separate. Long unbroken names wrap within the bubble. An incoming code
  request requiring a destination opens file review. The panel-first hidden
  default remains intact.
- Primary active/hover button backgrounds use #1c71d8/#1a65c2 with white text;
  the active contrast is approximately 4.77:1.

## Current evidence

`cargo check --workspace --all-targets` and
`cargo clippy --workspace --all-targets --no-deps -- -D warnings` passed after
integration. The current native GTK regression passed in a separate D-Bus/Xvfb
session using `app/linuxdrop/tests/run-native-regressions.sh`. It exercises real
GTK widgets and a deliberately delayed fixture service: draft additions, protocol
reordering, focused widget retention, file validation, dirty settings and incoming
Quick Share subset plus exact destination. The fixture is test-only; real HTTPS
and daemon consent are checked separately in `COMPLETION_DAEMON_2026-10-06.md`.

The consolidated `cargo test --workspace` run passed 30 tests, with this one
display-dependent GTK test correctly skipped by that headless command and passed
separately through the native runner. No hardware WPS/PHY test is implied by the
new helper compiling or its credential-validation unit test passing.

Root inspected the 480x600 Send/Settings and wide Send captures under
`docs/acceptance/ui/completion/`. Root also inspected
`shell-verification-review.png` from the desktop owner's isolated GNOME 46 runtime:
German confirmation text and a 100-character file name fit within the bubble.
The test device is explicitly labelled as a fixture, not a connected phone.

This is not a claim that every desktop/environment combination is accepted.
Fractional scaling, mixed-monitor hotplug, full screen-reader traversal, portal
exports and GNOME versions beyond the recorded runtime remain on the full goal's
acceptance ledger. New installable artifacts still require the final integration
build; old 0.1.0 packages do not contain this pass.

## Download-offer reception follow-up

Transfers now includes “Receive from a link” / “Über einen Link empfangen”. The
native dialog discloses local HTTP, retains the address after validation failure
and leads to the existing PIN and file-selection review. PIN text distinguishes
incoming download offers from outgoing uploads. The isolated GTK regression and
the real daemon reverse-download integration passed. Root inspected the fully
opened German dialog at 480x600 in
`ui/completion/download-offer-review.png`; all text, input and actions fit.
The visible desktop demo was not restarted. Existing broader desktop acceptance
items above remain open.
