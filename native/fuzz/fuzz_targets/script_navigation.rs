//! A script's request to go somewhere, as the trusted core follows it
//! (script-api.md section 3.3; `neoteletext::links::Links::
//! script_navigation`): the target is the script's own text, carried to
//! the core in a report. Whatever it is, nothing panics; another channel
//! is never followed without the viewer's activation; a script never
//! hands the viewer to another program or raises a prompt; and a page it
//! goes to is a page a channel may have.
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext::address;
use neoteletext::links::{Channel, Follow, Links};
use neoteletext_core::limits::{FIRST_PAGE_ADDRESS, LAST_PAGE_ADDRESS};

/// The channels a script runs in: a web channel's folder and a file.
const CHANNELS: [&str; 2] = ["https://example.test/chan/", "file:///home/viewer/chan.ttx"];
/// The first and last page numbers a channel may have.
const PAGES: core::ops::RangeInclusive<u16> = FIRST_PAGE_ADDRESS..=LAST_PAGE_ADDRESS;

fuzz_target!(|data: &[u8]| {
    let Some((&flags, rest)) = data.split_first() else { return };
    let Ok(target) = std::str::from_utf8(rest) else { return };
    let Ok(own) = address::parse(CHANNELS[usize::from(flags & 1)]) else { return };
    let activated = flags & 2 != 0;
    let channel = Channel { id: 1, address: &own };
    let Ok(follow) = Links::default().script_navigation(target, &channel, activated, 10_000) else { return };
    match follow {
        Follow::Page(page) | Follow::SubPage(page, _) => assert!(PAGES.contains(&page), "{target:?} went to page {page:X}"),
        Follow::InitialPage => {}
        Follow::Channel(_) => assert!(activated, "{target:?} went to another channel without the viewer"),
        Follow::Ask(_) | Follow::HandOff(_) => panic!("{target:?} made {follow:?}"),
    }
});
