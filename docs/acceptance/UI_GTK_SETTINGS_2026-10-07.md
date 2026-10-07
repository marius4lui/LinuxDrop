# GTK settings write feedback — 2026-10-07

Scope: the general settings form and reset outcome handling. No browser,
live demo restart, installed-package update, or physical protocol test.

Settings now show a pending row while saving and disable the affected control.
An unsuccessful or unconfirmed receipt restores confirmed switch/choice values,
retains text/numeric drafts, and leaves a persistent, translated retry action next
to that setting. Retry uses the latest edited text. Restoring a saved text value
clears its error without another write. Error text and folder paths are literal.
Search/category filtering includes the status row without losing the query.

Queued writes check service readiness, captured owner and generation before IPC;
obsolete-owner receipts cannot confirm a save. A successful receipt updates the
form immediately. A snapshot begun before that receipt is discarded and fetched
again, so it cannot revert the confirmed choice.

Reset refuses pending field writes; while reset is in flight, settings controls
are disabled and new writes are blocked. Drafts clear only after a successful
reset receipt from the same owner. Failure preserves them. Controls unlock when
the IPC completes or times out (30 seconds maximum), including when the owner
changed; this pass does not claim immediate reset recovery on owner loss.

Native regression coverage uses the existing private D-Bus/Xvfb runner, with
isolated runtime/config/data/cache and LinuxDrop activation excluded. It checks
pending state, failed rollback without duplicate writes, exact retry, edits made
after failure, unchanged-value error dismissal, late snapshot rejection, failed
reset preserving drafts, reset/write serialization, and offline proxy refusal
followed by recovery. GTK's accessibility test backend verifies the translated,
setting-specific retry name, and the button accepts keyboard focus.

The German 480×600 rendering was inspected in
[settings-save-failed.png](ui/completion/settings-save-failed.png). The synthetic
backend error deliberately remains English; app guidance and actions are German.
The error wraps and the retry button remains visible without horizontal clipping.

Validation: `cargo test -p linuxdrop --no-run`, the isolated native scenario, and
`cargo clippy -p linuxdrop --all-targets -- -D warnings` passed. Real screen-reader
announcements, mixed-DPI hardware and installed-session acceptance remain open.
