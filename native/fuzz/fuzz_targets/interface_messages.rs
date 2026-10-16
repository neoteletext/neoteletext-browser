//! What the trusted core reads from the interface process, which is
//! untrusted (`architecture.md` section 6.4).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    while let Ok(Some(message)) = protocol::read_from_interface(&mut input) {
        core::hint::black_box(message);
    }
});
