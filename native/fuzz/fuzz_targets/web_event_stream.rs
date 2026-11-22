//! An event stream as the web client reads it (the HTML standard, section
//! 9.2.6): whatever the bytes, each event keeps to the bounds of the
//! message that carries it, and however the bytes are cut, the events
//! are the same.
#![no_main]

use libfuzzer_sys::fuzz_target;
use protocol::web::{MAX_EVENT_DATA, MAX_EVENT_ID, MAX_EVENT_TYPE};

fuzz_target!(|data: &[u8]| {
    let Some((&cut, stream)) = data.split_first() else { return };
    let whole = fetcher::web_fuzzing::events(stream, stream.len());
    let pieces = fetcher::web_fuzzing::events(stream, usize::from(cut) + 1);
    assert_eq!(whole, pieces, "the same events however the bytes are cut");
    for (kind, data, last_id) in whole.unwrap_or_default() {
        assert!(!kind.is_empty() && kind.len() <= MAX_EVENT_TYPE && data.len() <= MAX_EVENT_DATA && last_id.len() <= MAX_EVENT_ID);
        assert!(!last_id.contains('\0'));
    }
});
