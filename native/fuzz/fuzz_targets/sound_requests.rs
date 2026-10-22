//! What the sound helper reads from the trusted core (decision 55).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    while let Ok(Some(message)) = protocol::sound::read_to_sound(&mut input) {
        core::hint::black_box(message);
    }
});
