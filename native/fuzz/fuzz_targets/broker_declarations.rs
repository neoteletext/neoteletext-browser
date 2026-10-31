//! What the broker reads from the interface on their own channel (DEC-1551
//! item 3), which it does not trust: a kind byte, then the payload.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Some((&kind, payload)) = data.split_first()
        && let Ok(declaration) = confine::broker::decode_declaration(kind, payload)
    {
        assert!(declaration.paths.len() <= confine::broker::MAX_FILES_AT_ONCE);
    }
});
