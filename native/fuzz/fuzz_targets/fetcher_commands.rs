//! What a fetcher reads from the trusted core: input messages, whole items
//! of the input stream of at most 4 096 bytes, which it writes to a
//! channel's WebSocket (REQ-INT-133), a move marked as one contact's move
//! alone (REQ-RCH-035); the answers to its cookie questions, each within
//! its bound (decision 62); and what it is asked to fetch or measure.
#![no_main]

use libfuzzer_sys::fuzz_target;
use protocol::fetcher::{MAX_COOKIE_HEADER, ToFetcher, read_to_fetcher};

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    while let Ok(Some(message)) = read_to_fetcher(&mut input) {
        match message {
            ToFetcher::Send { bytes, move_of, .. } => {
                assert!(bytes.len() <= neoteletext_input::MAX_MESSAGE);
                let mut items = Vec::new();
                assert!(neoteletext_input::event::read_message(&bytes, &mut |item| items.push(format!("{item:?}"))).is_ok());
                if let Some(contact) = move_of {
                    assert!(usize::from(contact) < neoteletext_input::MAX_CONTACTS);
                    assert_eq!(items.len(), 1, "a move alone");
                }
            }
            ToFetcher::CookieAnswer { header, .. } => assert!(header.len() <= MAX_COOKIE_HEADER),
            ToFetcher::ReachedAnswer { .. } | ToFetcher::FetchSubPages { .. } | ToFetcher::FetchSubPage { .. } | ToFetcher::MeasureRoundTrip(_) => {}
        }
    }
});
