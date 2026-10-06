#![no_main]
libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Some((&mode, archive)) = data.split_first() {
        let _ = linuxdrop_airdrop::fuzzing::archive_input(archive, mode & 1 == 1);
    }
});
