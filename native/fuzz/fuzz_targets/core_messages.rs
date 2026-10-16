//! What the interface process reads from the trusted core.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    while let Ok(Some(message)) = protocol::read_from_core(&mut input) {
        core::hint::black_box(message);
    }
});
