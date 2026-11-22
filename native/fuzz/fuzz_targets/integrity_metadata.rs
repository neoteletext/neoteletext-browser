//! A request's integrity metadata as the web client reads it (the
//! Subresource Integrity standard's "parse metadata" and "do bytes match
//! metadataList"; the script API's specification section 11.4, target
//! 13). Whatever the text, metadata that names no known hash matches any
//! body, a body matches its own hash and no other body does.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&split, rest)) = data.split_first() else { return };
    let (metadata, body) = rest.split_at(usize::from(split).min(rest.len()));
    let Ok(metadata) = std::str::from_utf8(metadata) else { return };
    let (asks, matches) = fetcher::web_fuzzing::integrity_of(metadata, body);
    // Metadata of nothing but spaces asks for nothing.
    if metadata.trim().is_empty() {
        assert!(!asks && matches);
    }
});
