# Shared hardware status - 2026-10-07

HardwareStatus reuses the passive inventory's actual Radio, NetworkInterface,
BluetoothController and capability/evidence models. RadioView adds a typed
AirDrop / Quick Share / Not reserved annotation from the daemon's leases.
Daemon Data stores this model directly; JSON indexing no longer constructs its
hardware state or USB arrival comparisons. The initial empty inventory is a
valid typed value with observation time zero.

Snapshot validation in Rust/GTK and the generated GNOME schema now checks the
hardware fields, capability enums, reservation enum and integer ranges. Missing
safety fields, duplicate/empty device identifiers, and an unprotected radio with
an associated active/default-route interface are rejected. Unknown capability
remains unknown; driver identity is not promoted to proven injection or AWDL.
Optional driver details preserve compatibility with the inventory's existing
serde defaults. GTK links these data models but does not invoke passive probing.

The redacted diagnostic export previously indexed a nonexistent top-level
firmware field. It now reads radio.driver_details.firmware from the typed radio.
The rest of the diagnostic response envelope and helper test/recovery response
models remain separate follow-up work.

Evidence completed in this pass:

- Four IPC payload tests pass, including preserved firmware/evidence, required
  protection/capability fields, invalid types/enums, duplicate radio and active
  interface protection. The same explicit hardware fixture is shared with GTK.
- Pure GJS reader passes 141 assertions, now including hardware firmware,
  missing safety fields, capability/reservation values, Bluetooth u8 overflow,
  duplicate radios and active-interface protection.
- Native GTK private-bus/Xvfb scenario passes 1/1 in 22.02 seconds, including
  hardware preference/failure/retry, blocked adapter, details and daemon-owner
  replacement. No public/live service activation occurs.
- Native GNOME 46 / Mutter 46.2 Shell and preferences smoke passes at German
  150 percent text and 800x600 with the generated hardware schema.
- App, daemon and IPC all-target Clippy with schema/client features passed;
  formatting and diff checks passed.

No physical USB/radio capability or Apple/Android interoperability is proved by
these fixtures. The Ubuntu package verification separately exercises real
D-Bus/HTTPS status, including its virtual network inventory, with both typed
Rust and GJS readers. The live demo is not restarted and no browser is used.
