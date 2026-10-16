//! What the trusted core reads from a fetcher process, which talks to the
//! network and is untrusted.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    while let Ok(Some(message)) = protocol::fetcher::read_from_fetcher(&mut input) {
        core::hint::black_box(message);
    }
});
