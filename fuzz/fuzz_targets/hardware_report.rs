#![no_main]
libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let (_, _, combinations) = linuxdrop_hardware::parse_iw(text);
        let _ = linuxdrop_hardware::details::parse_combinations(&combinations);
        let _ = linuxdrop_hardware::details::parse_ethtool(text);
    }
});
