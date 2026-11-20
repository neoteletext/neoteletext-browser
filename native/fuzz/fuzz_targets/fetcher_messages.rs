//! What the trusted core reads from a fetcher process, which talks to the
//! network and is untrusted: a part of a page's list of sub-pages among it,
//! held to a part's shape (REQ-1888). The same bytes are read once as one
//! tab's messages and once as a process's, each after its tab's number;
//! and whole messages named with a tab read back as that tab's (REQ-6980).
#![no_main]

use libfuzzer_sys::fuzz_target;
use protocol::fetcher::{FromFetcher, MAX_LIST_PARTS, MAX_LIST_VERSION, ROWS_PER_PART, name_tab, read_from_fetcher, read_from_fetcher_for};

/// Holds one message to what its reader promises.
fn check(message: FromFetcher) {
    {
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
}

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    let mut count = 0_usize;
    while let Ok(Some(message)) = read_from_fetcher(&mut input) {
        check(message);
        count += 1;
    }
    let mut input = data;
    while let Ok(Some((_, message))) = read_from_fetcher_for(&mut input) {
        check(message);
    }
    // Naming takes whole frames, whatever they hold, and never more than
    // was written; what it leaves is the frame cut short.
    let (mut written, mut named) = (data.to_vec(), Vec::new());
    name_tab(7, &mut written, &mut named);
    assert!(written.len() <= data.len());
    let mut input = named.as_slice();
    let mut read = 0_usize;
    while let Ok(Some((tab, message))) = read_from_fetcher_for(&mut input) {
        assert_eq!(tab, 7);
        check(message);
        read += 1;
    }
    assert!(read >= count, "every message that read alone reads named");
});
