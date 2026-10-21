//! What a fetcher reads from the trusted core: only input messages, whole
//! items of the input stream of at most 4 096 bytes, which it writes to a
//! channel's WebSocket (REQ-INT-133).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    while let Ok(Some(protocol::fetcher::ToFetcher::Send(message))) = protocol::fetcher::read_to_fetcher(&mut input) {
        assert!(message.len() <= neoteletext_input::MAX_MESSAGE);
        assert!(neoteletext_input::event::read_message(&message, &mut |_| {}).is_ok());
    }
});
