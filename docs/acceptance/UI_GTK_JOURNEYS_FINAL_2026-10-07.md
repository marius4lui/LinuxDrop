# GTK file journey completion - 2026-10-07

One bounded final review covered file selection/removal, recipient and protocol
selection, incoming consent/SAS, progress and received-file actions. Existing
coverage and prior hardware/settings acceptance were reviewed before editing.

Three remaining usability defects were corrected:

- Removing a file now focuses the next file action, the previous action at the
  end, or Choose files after the final removal. Keyboard users can continue
  without losing their place when an invalid selection is removed.
- Incoming destination paths are assigned only after disabling markup. Received
  filenames follow the same construction order, preventing transient parsing
  warnings for literal angle brackets and ampersands.
- Actual file-picker failures now show the existing error toast. User dismissal
  and cancellation remain silent. No new English/German catalog text was needed.

The existing private D-Bus/Xvfb scenario was extended, not duplicated. It now
activates the actual removal buttons and asserts neighboring and final-file
focus, checks the literal destination `/tmp/Reviewed <Sender> & files`, and
verifies the exact destination survives consent failure/retry and reaches IPC.
Its existing send/protocol, SAS subset, progress and saved-file checks also pass.

Validation: `cargo test -p linuxdrop --no-run`,
`app/linuxdrop/tests/run-native-regressions.sh` (one scenario, 17 seconds),
`cargo clippy -p linuxdrop --all-targets -- -D warnings`, and `git diff --check`
passed. The test used isolated runtime/config/data/cache, a private session bus
excluding installed LinuxDrop activation, and Xvfb. Portal/PipeWire and bus
shutdown messages were environmental; no GTK markup warnings occurred.

The compact German [incoming review](ui/journey-final/incoming-verification-review.png)
was visually inspected: the destination remains literal and the SAS, selected
files and accept/cancel actions are readable. The long confirmation content uses
the existing dialog scrolling. Keyboard focus is supported by widget assertions.

The file-picker failure branch was reviewed and compiled but no failing real
desktop portal was induced. Physical cross-device transfers, registered-app
launching and real screen-reader traversal remain unverified. The live demo was
neither modified nor restarted; no browser was used.
