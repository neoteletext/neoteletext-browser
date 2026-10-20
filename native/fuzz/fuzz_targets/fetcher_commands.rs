//! What a fetcher reads from the trusted core: only key records, which it
//! writes to a channel's WebSocket (requirement 18.4).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    while let Ok(Some(protocol::fetcher::ToFetcher::Send(record))) = protocol::fetcher::read_to_fetcher(&mut input) {
        assert!(record.len() <= neoteletext_input::key::MAX_RECORD);
        assert!(neoteletext_input::key::read(&record).is_ok());
    }
});
