//! Addresses typed into the trusted strip, or given on the command line.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = core::str::from_utf8(data) else { return };
    core::hint::black_box(neoteletext::address::parse(text).is_ok());
});
