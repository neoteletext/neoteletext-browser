//! An answer's coded body as the web client's body reader takes it (the
//! Fetch standard's "handle content codings"; neoteletext-common's
//! DEC-7260 item 7; `fetcher::web_fuzzing::body_ratio`, over
//! `fetch::Reader::piece` with no connection): the decoders, the bound on
//! a decoded body and the ratio of decoded to coded bytes that stops a
//! decompression bomb. The opening asserts each piece's bound, the whole
//! body's and the ratio past its floor; this target gives it every coding
//! and arbitrary bytes, and its seeds are each coding's bombs.
#![no_main]

use libfuzzer_sys::fuzz_target;

/// The codings an answer may name: those decoded, one that is not, and
/// the identity.
const CODINGS: [&[u8]; 5] = [b"gzip", b"deflate", b"br", b"identity", b"zstd"];

fuzz_target!(|data: &[u8]| {
    let Some((&coding, coded)) = data.split_first() else { return };
    let coding = CODINGS[usize::from(coding) % CODINGS.len()];
    core::hint::black_box(fetcher::web_fuzzing::body_ratio(coding, coded));
});
