# Bounded production-parser fuzzing

Install `cargo-fuzz` and a Rust nightly toolchain, then run from the repository:

```
cargo +nightly fuzz run localsend_offer -- -max_total_time=30 -max_len=65536 -rss_limit_mb=1024
cargo +nightly fuzz run airdrop_archive -- -max_total_time=30 -max_len=65536 -rss_limit_mb=1024
cargo +nightly fuzz run hardware_report -- -max_total_time=30 -max_len=65536 -rss_limit_mb=1024
```

Targets invoke production validation/parsers. AirDrop uses anonymous temporary storage and a 64-KiB decompression limit. No target opens sockets, changes radios or invokes privileged helpers. CI smoke runs are bounded regression exploration, not proof that the protocol parser is free of all faults. Save a minimized reproducer as a regular regression test when fixing a finding.
