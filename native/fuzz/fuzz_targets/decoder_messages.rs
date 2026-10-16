//! What the trusted core reads from a decoder process, which parses
//! untrusted teletext data and is itself untrusted.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    while let Ok(Some(message)) = protocol::decoder::read_from_decoder(&mut input) {
        core::hint::black_box(message);
    }
});
