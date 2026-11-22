//! A WebSocket server's frames as the web client reads them (RFC 6455
//! section 5; `fetcher::web_fuzzing::frames`, over `socket::Reading::
//! take` and the engine's frame reader), fed in pieces of a size the
//! input chooses. Whatever the bytes, nothing panics; every piece and
//! message keeps to its bound and no message is left without its last
//! piece (asserted inside the opening); and however the bytes are cut,
//! a script is told the same messages and the same end.
//!
//! Not reached: the socket's command side (`SocketSend`, `SocketClose`,
//! `MAX_QUEUED_BYTES`, `CLOSE_WAIT`) and its interim-answer loop.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&cut, bytes)) = data.split_first() else { return };
    let whole = fetcher::web_fuzzing::frames(bytes, bytes.len().max(1));
    let pieces = fetcher::web_fuzzing::frames(bytes, usize::from(cut) + 1);
    assert_eq!(whole, pieces, "the same frames told differently when cut in pieces of {}", usize::from(cut) + 1);
});
