//! What a fetcher reads from the trusted core: input messages, whole items
//! of the input stream of at most the input protocol's largest message,
//! which it writes to a channel's WebSocket (REQ-INT-133), and under push
//! protocol 9 never a viewer's input outside a `RULE_EVENT`
//! (neoteletext-common's page-rules.md RULE-46); viewer state's answers;
//! what it is asked to fetch or measure, a part of a page's list among
//! them; and a tab's joining and leaving its process. The same bytes are
//! read once as one tab's messages and once as a process's, each after its
//! tab's number (REQ-6980).
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext_input::event::{Item, is_page_input, read_message};
use protocol::fetcher::{MAX_LIST_PARTS, ToFetcher, read_to_fetcher, read_to_fetcher_for};

/// Holds one message to what its reader promises.
fn check(message: ToFetcher) {
    {
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
            // A script is asked for by its name, never by an address (the
            // script API's specification section 12.5): the reader takes
            // only a name's characters, so none can leave the channel's
            // folder.
            ToFetcher::FetchScript { name, .. } => {
                assert!(!name.is_empty() && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')), "{name:?}");
            }
            ToFetcher::StateAnswer { .. }
            | ToFetcher::StateSendOffer
            | ToFetcher::StateSendNone(_)
            | ToFetcher::FetchContents
            | ToFetcher::FetchPage { .. }
            | ToFetcher::FetchSubPage { .. }
            | ToFetcher::MeasureRoundTrip(_)
            | ToFetcher::TabOpens
            | ToFetcher::TabLeaves => {}
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    while let Ok(Some(message)) = read_to_fetcher(&mut input) {
        check(message);
    }
    let mut input = data;
    while let Ok(Some((_, message))) = read_to_fetcher_for(&mut input) {
        check(message);
    }
});
