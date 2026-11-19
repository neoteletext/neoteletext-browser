//! What the trusted core reads from a fetcher in snapshot mode, and what
//! such a fetcher reads from the trusted core (neoteletext-common's
//! `docs/tab-thumbnails.md` section 9, REQ-6884 item 2): every piece is
//! within one message and not empty, a denial's status is an HTTP status,
//! and the one message to the fetcher is `Close`.
#![no_main]

use libfuzzer_sys::fuzz_target;
use protocol::snapshot::{FromSnapshot, SnapshotEnd, ToSnapshot, read_from_fetcher, read_to_fetcher};

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    while let Ok(Some(message)) = read_from_fetcher(&mut input) {
        match &message {
            FromSnapshot::Piece(bytes) => assert!(!bytes.is_empty() && bytes.len() <= protocol::MAX_PAYLOAD),
            FromSnapshot::Ended(SnapshotEnd::Denied(status)) => assert!((100..=599).contains(status)),
            FromSnapshot::Confined | FromSnapshot::Ended(_) => {}
        }
        core::hint::black_box(message);
    }
    let mut input = data;
    while let Ok(Some(message)) = read_to_fetcher(&mut input) {
        assert_eq!(message, ToSnapshot::Close);
    }
});
