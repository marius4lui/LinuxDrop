# Shared Manager1 wire contract - 2026-10-07

The actual daemon introspection exposed 31 methods but omitted Changed even
though the daemon emitted it. Changed is now a declared zbus signal and emission
uses the generated signal method, making its uint64 revision visible to clients.

The new linuxdrop-ipc workspace crate contains the canonical Manager1 XML and
shared service/object/interface identities. One generator produces the Rust method
enum/signatures, an optional typed zbus proxy and GNOME's interface description.
The desktop supplies this XML to GDBus and checks argument/result signatures;
GNOME supplies the same XML and rejects unknown methods and wrong argument types
before dispatch. Portal calls through the GTK helper keep their own interfaces.
Native packages ship the XML; the RPM file manifest includes it. Existing Manager1
wire signatures and service names are retained.

The generator's --check mode is a CI gate. Daemon integration compares all live
method/signal argument signatures with the canonical XML, subscribes to an actual
Changed(t), checks a new revision, and rejects a wrong-type mutation. A typed
Rust client probe checks snapshots/defaults, signal decoding, invalid visibility
and empty file descriptor batches against the same isolated daemon. This probe
requires an explicit test environment flag and is not a production CLI.

Native GTK regression passed in 19.21 seconds, including rejection of unknown or
wrong-type calls before the service handler runs. GNOME Shell 46's private
800x600/150-percent German scenario and preferences test passed with equivalent
invalid-action assertions, owner replacement, stale callback and action feedback
coverage. Live demo untouched. Both app/daemon and optional Rust proxy compile;
final all-target Clippy passes with warnings denied. The release daemon passes
the full live introspection comparison, typed proxy probe and existing D-Bus/HTTPS
consent, settings, download-offer and shutdown integration.

JSON strings still carry settings, snapshots and diagnostics. This wire contract
does not claim complete typing or field-level validation of those payloads;
that remaining IPC work stays in the completion ledger. Packaged-daemon/live
contract results belong in dist/BUILD_REPORT_0.1.0+review.20261007.24.json.

Follow-up: shared Rust [settings/status payload types](IPC_PAYLOAD_TYPES_2026-10-07.md)
now validate GTK inputs and construct daemon status. Hardware/diagnostics and
GNOME JSON field validation remain separate work.
