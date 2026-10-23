//! A release file's gzip member, downloaded by the updater
//! (`installation-and-updates.md` section 6): whatever it holds, reading
//! it never writes beyond the length asked for, and a member the release
//! tool writes reads back to its file.
#![no_main]

use libfuzzer_sys::fuzz_target;

/// The most a fuzzed member may say it decompresses to: room for the
/// fuzzer's inputs and their expansion.
const MAX_OUTPUT: usize = 1 << 20;

fuzz_target!(|data: &[u8]| {
    let Some((&length, member)) = data.split_first() else { return };
    let length = usize::from(length) * 64;
    if let Ok(out) = update::gzip::decompress(member, length) {
        assert_eq!(out.len(), length);
    }
    if let Ok(out) = update::gzip::decompress_at_most(member, MAX_OUTPUT) {
        assert!(out.len() <= MAX_OUTPUT);
    }
    assert_eq!(update::gzip::decompress(&update::gzip::compress(member), member.len()).as_deref(), Ok(member));
});
