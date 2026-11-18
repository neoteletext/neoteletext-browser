//! What a fetcher reads from the trusted core: input messages, whole items
//! of the input stream of at most the input protocol's largest message,
//! which it writes to a channel's WebSocket (REQ-INT-133), and under push
//! protocol 9 never a viewer's input outside a `RULE_EVENT`
//! (neoteletext-common's page-rules.md RULE-46); viewer state's answers;
//! and what it is asked to fetch or measure, a part of a page's list among
//! them.
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext_input::event::{Item, is_page_input, read_message};
use protocol::fetcher::{MAX_LIST_PARTS, ToFetcher, read_to_fetcher};

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    while let Ok(Some(message)) = read_to_fetcher(&mut input) {
        match message {
            ToFetcher::Send { bytes, move_of, .. } => {
                assert!(bytes.len() <= neoteletext_input::MAX_MESSAGE);
                let mut items = 0_usize;
                let read = read_message(&bytes, &mut |item: Item<'_>| {
                    // What reads at the top level is never a viewer's input.
                    assert!(!is_page_input(&item), "bare input passed the reader");
                    items = items.saturating_add(1);
                });
                assert!(read.is_ok());
                if let Some(contact) = move_of {
                    assert!(usize::from(contact) < neoteletext_input::MAX_CONTACTS);
                    assert_eq!(items, 1, "a move alone");
                }
            }
            // A part of a page's list of sub-pages, `NNN/P.sub`, that a list
            // can have (REQ-1888).
            ToFetcher::FetchSubPages { part, .. } => assert!(usize::from(part) < MAX_LIST_PARTS),
            ToFetcher::StateAnswer { .. }
            | ToFetcher::StateSendOffer
            | ToFetcher::StateSendNone(_)
            | ToFetcher::FetchContents
            | ToFetcher::FetchPage { .. }
            | ToFetcher::FetchSubPage { .. }
            | ToFetcher::MeasureRoundTrip(_) => {}
        }
    }
});
