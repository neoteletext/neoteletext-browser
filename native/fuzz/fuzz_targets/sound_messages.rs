//! What the trusted core reads from the sound helper, which is untrusted
//! (decision 55).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    while let Ok(Some(message)) = protocol::sound::read_from_sound(&mut input) {
        core::hint::black_box(message);
    }
});
