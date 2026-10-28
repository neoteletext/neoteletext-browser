//! What a fetcher reads from the trusted core: input messages, whole items
//! of the input stream of at most 4 096 bytes, which it writes to a
//! channel's WebSocket (REQ-INT-133); and the answers to its cookie
//! questions, each within its bound (decision 62).
#![no_main]

use libfuzzer_sys::fuzz_target;
use protocol::fetcher::{MAX_COOKIE_HEADER, ToFetcher, read_to_fetcher};

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    while let Ok(Some(message)) = read_to_fetcher(&mut input) {
        match message {
            ToFetcher::Send(message) => {
                assert!(message.len() <= neoteletext_input::MAX_MESSAGE);
                assert!(neoteletext_input::event::read_message(&message, &mut |_| {}).is_ok());
            }
            ToFetcher::CookieAnswer { header, .. } => assert!(header.len() <= MAX_COOKIE_HEADER),
            // A yes or no; a page; a page and a sub-code: each checked by
            // the reader this runs.
            ToFetcher::ReachedAnswer { .. } | ToFetcher::FetchSubPages { .. } | ToFetcher::FetchSubPage { .. } => {}
        }
    }
});
