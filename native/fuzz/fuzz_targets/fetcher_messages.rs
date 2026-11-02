//! What the trusted core reads from a fetcher process, which talks to the
//! network and is untrusted: a part of a page's list of sub-pages among it,
//! held to a part's shape (REQ-1888).
#![no_main]

use libfuzzer_sys::fuzz_target;
use protocol::fetcher::{FromFetcher, MAX_LIST_PARTS, MAX_LIST_VERSION, ROWS_PER_PART, read_from_fetcher};

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    while let Ok(Some(message)) = read_from_fetcher(&mut input) {
        match &message {
            FromFetcher::SubPages { version, part, .. } => {
                assert!(version.len() <= MAX_LIST_VERSION);
                assert!(part.rows.len() <= ROWS_PER_PART && usize::from(part.first) % ROWS_PER_PART == 0);
                assert!(usize::from(part.first) + part.rows.len() <= usize::from(part.count));
            }
            FromFetcher::SubPagesMissing { part, .. } | FromFetcher::SubPagesFailed { part, .. } => assert!(usize::from(*part) < MAX_LIST_PARTS),
            _ => {}
        }
        core::hint::black_box(message);
    }
});
