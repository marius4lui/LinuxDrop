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


## Focused Astra UX/UI follow-up ? 2026-10-07

Two Astra/high agents owned separate native-dialog/settings and Shell/style areas.
Changes preserve the panel-first flow and leave the live demo untouched:

- Failed incoming consent reopens the review with the same destination, selected
  subset and collision rule. The error is visible without the D-Bus namespace.
- Receiving PIN setup precedes the dependent switch; search focus and unsaved
  entries survive rebuilds. Offline switch/choice edits restore persisted values.
- Link/PIN dialogs have entry focus and Enter actions; folder-picker failures are
  reported while ordinary dismissal remains quiet.
- The Shell bubble replaces stale actions when offline, disables duplicate
  mutations during requests, preserves progress actors/focus and restores panel
  focus on close. Its scroll area, labels and progress track use allocated space.
- Essential secondary GTK instructions have higher contrast. Symlink selections
  receive a specific explanation matching daemon validation.

Evidence: 48 workspace tests pass (three environment-specific scenarios skipped
by that command), warning-free all-target workspace Clippy, and the separately run
native GTK regression passes. The native scenario explicitly injects an incoming
consent error, checks the retained choices and verifies a successful retry; it
also checks PIN dependencies and settings search focus. Updated 480x600 captures
were inspected after dialog animations settled, including
`ui/completion/incoming-retry-review.png`.

The real daemon D-Bus/HTTPS integration also passes. Source-descriptor regressions
exercise LocalSend, Quick Share, AirDrop and reverse downloads after replacing the
original path; they receive the original bytes. This pins file identity and does
not promise an immutable snapshot during concurrent in-place writes.

A separate, temporary GNOME 46 profile/private bus passes the reproducible
`extensions/gnome-shell/tests/run-bubble-smoke.sh` scenario with normal text and
German 150% text: hidden/open, native scroll, verification, transfer navigation,
progress geometry, busy/offline actions and focus. This uncovered and fixed a real
progress allocation bug. These checks do not claim physical mixed-DPI, complete
screen-reader or later GNOME-version acceptance. Installed demo/packages remain
unchanged and do not yet contain these source changes.


Root also inspected the final German 150% text capture
`ui/completion/shell-verification-large-review.png`. The full confirmation and
secondary actions fit; long peer/file names use intentional ellipsis. Relative
font sizes now respect the system text setting. The final Shell regression also
changes each file's byte progress and verifies stable Quick Settings device
actors, covering the two focus regressions found during root review.


## Portal descriptor handoff ? 2026-10-07

The native app now passes opened regular files with `PrepareSendFiles`, using
batches of at most 16 Unix descriptors. Combined draft count/size rules apply to
all batches; each append is atomic. A failed preparation or failed start/offer
releases its partial draft. The daemon also expires abandoned descriptors on its
maintenance tick. Host-path PrepareSend stays available for existing clients.

Passed: targeted daemon tests, all-target workspace Clippy and the isolated GTK
regression including descriptor validation, 17-file client batching and cleanup
when the next file cannot be opened. The new
`tests/integration/run-fd-portal.sh` uses a private network namespace and bus with
an actual document portal. It exports and revokes a document, closes client file
handles, replaces the original path and receives all 25 offered files byte-for-byte
from the retained descriptors. Write-only descriptors, directories, pipes, invalid
names and combined-count overflow are rejected without partially appending a batch.
No installed Flatpak or physical device acceptance is inferred from this test.

## 2026-10-07: saved settings and effective service status

The Settings page now keeps a persistent status card above search. It distinguishes
saved choices while services apply, enabled services that failed or lack a usable
network, and the last-known settings shown while disconnected. Details opens the
Hardware page; successful recovery removes the warning. Backend-only updates do
not rebuild the settings form or erase the search query.

The native private-session GTK regression passed in German after exercising
applying -> error -> offline -> recovered states, the Details navigation and
search-widget identity. The rendered 480x600 view was inspected:
`ui/completion/settings-apply-error.png`. All status text, actions, search and
bottom navigation fit. No running user demo was restarted.

## 2026-10-07: persistent link revocation

The Transfers page shows an active-link card with an explicit stop action and
explains that stopping cancels its downloads. It remains visible after a failed
stop so the user can retry; it disappears only when the daemon reports completed
cleanup. Restart/reset dialogs explain why a live link must be stopped first.
The private GTK regression tested stop failure then successful retry. The German
480x600 render `ui/completion/active-download-link.png` fits the text, button,
transfer progress and navigation without overlap.


## 2026-10-07: recoverable helper failure

Lost radio-helper ownership no longer leaves nearby devices or active transfers
shown as ready. The error explains that radio cleanup may still be running and
points to adapter reconnection and service restart in Settings. GTK and the Shell
bubble have the German translation; the Shell now translates terminal error text.
The isolated GNOME 46 smoke passed with German/150% text, including replacement
of Cancel with Details/Done after helper failure and the translated explanation.
This is runtime behavior acceptance, not a new rendered image or physical-radio
claim. The live demo remains unchanged.
