# GTK hardware inspection and consent — 2026-10-07

This focused pass fixes remaining Hardware page interactions. It does not restart
the live demo or exercise a physical adapter, browser, or installed app session.

Inventory and backend updates now preserve expanded adapters by identity and
restore keyboard action focus. A newly blocked radio still updates its disabled
test action. Adapter, driver, network and backend text is assigned after disabling
markup, so names containing angle brackets or ampersands remain literal without
construction-time GTK markup warnings. The test, preference and details actions
expose translated accessible names including their adapter name.

The active hardware test confirmation is tied to the current service owner and
generation. Service loss dismisses it; an obsolete confirmation cannot run against
a replacement owner. The asynchronous call checks that identity before submission
and discards a result from an obsolete owner.

The existing private D-Bus/Xvfb native scenario checks expansion and keyboard
focus across an inventory change, newly blocked radio sensitivity, literal adapter
text, all three accessible action names, confirmation dismissal, and an explicit
old confirmation response after a real private-bus owner replacement. The
replacement fixture accepts only snapshot calls, so obsolete actions fail the
test if they reach it. No active hardware diagnostic is executed by the fixture.

The German 480×600 [hardware render](ui/final-pass/hardware-inspection.png) was
inspected: literal names and the expanded adapter remain readable in the scrolling
page. Lower action rows are covered by widget assertions, not by that screenshot.
The capture includes a transient toast from an earlier test step.

Validation against the combined GTK changes (including the parent agent's numeric
settings correction):

- `cargo test -p linuxdrop --no-run` passed.
- `app/linuxdrop/tests/run-native-regressions.sh` passed its native scenario.
- `cargo clippy -p linuxdrop --all-targets -- -D warnings` passed.
- `git diff --check` passed for the working changes.

Native validation used isolated runtime/config/data/cache, a private session bus
excluding installed LinuxDrop service activation, and an Xvfb display. The final
run had no GTK markup warnings; unrelated portal/PipeWire and shutdown cleanup
messages remain environment noise. Real screen-reader traversal, mixed-DPI
hardware, physical diagnostics and protocol interoperability remain unverified.
