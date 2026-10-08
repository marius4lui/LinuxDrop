# Contributing

Read [INSTALL](docs/INSTALL.md) and [ARCHITECTURE](docs/ARCHITECTURE.md). Rust is pinned by `rust-toolchain.toml`; use the lockfiles. Run README's formatting, Clippy and relevant tests. UI changes require actual native screenshots and interaction checks.

Keep UI independent of protocol internals. Preserve explicit consent, terminal transfer states, private temporary storage and active-adapter protection. Never add synthetic discovery devices to the normal runtime or label simulated protocol evidence as phone interoperability.

Record vendor changes in `vendor/PROTOCOL_SOURCES.md` with purpose and regression coverage. Retain license notices. Release artifacts must carry a matching source archive and identify supported distribution/desktop versions.

Physical acceptance reports should identify OS/client versions, chipset/driver, channel/band, both transfer directions, cancellation/rejection, file hashes and whether Internet connectivity survived. Redact personal information.
