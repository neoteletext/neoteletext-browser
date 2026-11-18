//! `viewer-keys.bin` (neoteletext-common's `docs/viewer-state.md` VS-108),
//! which a program holding the sealing key, or a copied or altered profile,
//! could have written: whatever it holds, opened or not, reading it keeps
//! every bound and never panics. The input is tried both as the sealed file
//! and, sealed here, as the text inside, so the reader of the text is
//! reached past the cipher.
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext::viewer_keys::MOST_KEYS_PER_JAR;
use neoteletext::viewer_keys::file::{read_for_fuzzing, seal_for_fuzzing};

/// A sealing key, fixed so a run is repeatable.
const KEY: [u8; 32] = [0x5E; 32];
/// 2026-11-18, the time every key's limits are judged against.
const NOW: u64 = 1_794_960_000;

fuzz_target!(|data: &[u8]| {
    assert!(read_for_fuzzing(&KEY, data, NOW) <= MOST_KEYS_PER_JAR);
    if let Some(sealed) = seal_for_fuzzing(&KEY, data) {
        assert!(read_for_fuzzing(&KEY, &sealed, NOW) <= MOST_KEYS_PER_JAR);
    }
});
