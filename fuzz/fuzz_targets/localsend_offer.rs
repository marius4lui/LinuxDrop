#![no_main]
libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let _ = linuxdrop_localsend::fuzzing::validate_prepare_json(data);
});
