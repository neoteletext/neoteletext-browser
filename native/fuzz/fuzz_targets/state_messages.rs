//! Viewer state's messages on the fetcher's pipe (neoteletext-common's
//! `docs/viewer-state.md` VS-119), each way: what a fetcher, which talks to
//! the network and is untrusted, tells the trusted core, and what the core
//! tells a fetcher. The first byte picks one of the six kinds, so every run
//! reaches a state reader; whatever the rest is, a message read is of its
//! exact length and writes back the same bytes.
#![no_main]

use libfuzzer_sys::fuzz_target;
use protocol::fetcher::{read_from_fetcher, read_to_fetcher, write_to_core, write_to_fetcher};

/// The state messages' kinds: the fetcher's, then the core's (VS-119).
const KINDS: [u8; 6] = [170, 171, 172, 173, 174, 175];

fuzz_target!(|data: &[u8]| {
    let Some((&pick, payload)) = data.split_first() else { return };
    let kind = KINDS[usize::from(pick) % KINDS.len()];
    let mut framed = vec![kind];
    framed.extend_from_slice(&u32::try_from(payload.len()).unwrap_or(u32::MAX).to_be_bytes());
    framed.extend_from_slice(payload);
    if let Ok(Some(message)) = read_from_fetcher(&mut framed.as_slice()) {
        let mut written = Vec::new();
        write_to_core(&mut written, &message).unwrap();
        assert_eq!(written, framed, "a fetcher's state message writes back as read");
    }
    if let Ok(Some(message)) = read_to_fetcher(&mut framed.as_slice()) {
        let mut written = Vec::new();
        write_to_fetcher(&mut written, &message).unwrap();
        assert_eq!(written, framed, "the core's state message writes back as read");
    }
});
