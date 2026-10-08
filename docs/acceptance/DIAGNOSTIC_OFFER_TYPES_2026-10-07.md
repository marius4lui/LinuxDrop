# Diagnostic and download-offer contracts - 2026-10-07

Manager1's non-status JSON replies now have shared Rust models: full diagnostics,
redacted export, recovery status, active hardware report and download offer.
The privileged helper and user daemon reuse the same diagnostic step/report
records from linuxdrop-hardware. The daemon constructs typed response objects;
an unexpected recovery variant or helper error becomes an explicit D-Bus error.

GTK routes JSON responses through a single method-aware validator. Hardware
reports require actual steps and a radio ID; recovery requires its expected
status tag and typed ownership flags. Download offers require a numeric HTTP
socket endpoint, a nonzero port/lifetime, an ASCII PIN and encrypted=false,
matching the actual local offer service. This does not authenticate a recipient
or prove LAN reachability. The existing HTTP/PIN confirmation is unchanged.

A malformed successful offer reply can arrive after the server has created the
link. GTK now revokes that offer through the same current service owner before
allowing another creation and refreshes status for cleanup failure recovery.
No invalid URL/PIN dialog is displayed. ExportDiagnostics is projected through
an explicit field allowlist, including nested backend/radio fields, so unknown
additive fields cannot silently enter a report labelled redacted. Selected
field contents still come from the trusted local service; this is not a general
secret detector. DownloadOffer intentionally has no derived Debug output.

Read-only report requests ignore a late old-owner result, and displayed reports
participate in the existing service-loss dialog lifecycle.

Verification in this pass:

- Five IPC payload regressions pass. The new case covers malformed addresses,
  wrong PIN/flag types, zero lifetime/port, incomplete recovery tags, wrong
  diagnostic booleans, and removal of additive export fields.
- The native private-bus GTK scenario passes 1/1 in 22.49 seconds, including a
  malformed successful offer, no usable link/PIN dialog, actual fixture-side
  revocation and restored retry. The first assertion incorrectly required every
  dialog to be gone while the synthetic confirmation was still closing; it now
  checks the required absence of a usable offer dialog alongside revocation.
- App/daemon/helper/IPC all-target Clippy and formatting/diff checks passed.
- The typed real-daemon probe now also validates GetDiagnostics and
  ExportDiagnostics. Packaged D-Bus/HTTPS integration runs this probe and the
  existing GJS status reader against actual observed inventory and transfers.

No new GNOME rendering is needed: Shell code/schema is unchanged from revision
28. Active physical hardware diagnostic and real helper-recovery execution were
not induced by this payload refactor. The shared serde wire shape is preserved.
No browser or live demo restart is used.
