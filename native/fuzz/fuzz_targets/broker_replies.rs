//! What the trusted core reads from the broker (decision 56): a kind
//! byte, then the payload.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Some((&kind, payload)) = data.split_first() {
        let _ = core::hint::black_box(confine::broker::decode_reply(kind, payload.to_vec()));
    }
});
