//! An answer's coded body as the web client decodes it (gzip, deflate and
//! brotli; neoteletext-common's DEC-7260 item 7): whatever the bytes, a
//! step takes no more than it was given and writes no more than its room,
//! and however the bytes are cut, what they decode to is the same.
#![no_main]

use libfuzzer_sys::fuzz_target;

/// The most a fuzzed body is decoded to: room for the fuzzer's inputs and
/// their expansion.
const MOST: usize = 1 << 20;
const CODINGS: [&[u8]; 3] = [b"gzip", b"deflate", b"br"];

fuzz_target!(|data: &[u8]| {
    let [coding, cut, room, coded @ ..] = data else { return };
    let coding = CODINGS[usize::from(*coding) % CODINGS.len()];
    fetcher::web_fuzzing::gzip_header(coded);
    let whole = fetcher::web_fuzzing::decode(coding, coded, coded.len(), MOST, MOST);
    let pieces = fetcher::web_fuzzing::decode(coding, coded, usize::from(*cut) + 1, usize::from(*room) + 1, MOST);
    if let (Some(whole), Some(pieces)) = (&whole, &pieces)
        && whole.len() <= MOST
        && pieces.len() <= MOST
    {
        assert_eq!(whole, pieces, "the same bytes decode the same however they are cut");
    }
    assert_eq!(whole.is_some(), pieces.is_some(), "bytes are their coding's or not, however they are cut");
});
