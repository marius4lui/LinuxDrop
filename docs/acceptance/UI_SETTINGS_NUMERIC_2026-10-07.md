# Settings range and precision review - 2026-10-07

The parent compared `crates/linuxdrop-daemon/src/settings.rs` defaults and
validation with the GTK settings form. All user-facing keys have controls across
general, receiving, visibility, LocalSend, Quick Share, AirDrop, hardware,
Bluetooth, notifications, transfers/history, network and diagnostics.
`schema_version` remains internal metadata. This is source coverage, not a claim
of exhaustive installed-session acceptance for every option.

Two discrepancies affected valid saved values:

- Visibility accepts 0 through 1440 minutes. Zero disables the automatic timer;
  the old control clamped it to one minute. The control now includes zero and
  explains its meaning in English and German. Screen-lock hiding remains a
  separate setting.
- The receive limit accepts 1 through 10,995,116,277,760 bytes. The decimal-MB
  control now covers that complete range with six decimal places. Converting
  edits and retries back to bytes rounds floating-point representation error,
  preserving individual bytes. The field has room for the full maximum value.

The existing isolated native GTK scenario covers both bounds, the default
107,374,182,400-byte limit, zero-minute visibility, visiting/leaving a numeric
field without a settings write, and a deliberate one-byte edit reaching the
private D-Bus fixture unchanged. This avoids an unintended service restart just
because a user inspected an existing limit.

Native visual evidence: [compact German numeric field](ui/final-pass/settings-numeric-precision.png).
The screenshot is fixture evidence; it is not physical transfer acceptance.
Build, native regression and app Clippy results are recorded with the companion
hardware acceptance from this pass. The user's running demo was not restarted.
