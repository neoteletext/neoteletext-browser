//! A channel's own redirects, on a live channel: what the viewer's keys and
//! pages have to do with them needs the typing that a build may leave out.

use interface_core::canvas::PageKey;
use interface_core::library::MAX_TAB_ENTRIES;
use neoteletext_core::limits::SUBCODE_ANY;
use neoteletext_input::event::{Action, KeyCode};
use protocol::ToInterface;
use protocol::decoder::FromDecoder;
use protocol::fetcher::Entry;
use protocol::interaction::{InputIntent, Intent};
use protocol::subpages::SubPageStep;

use super::super::super::test_support::Test;
use super::*;
use crate::session::Went;

const LIVE: &str = "https://live.invalid/channel/";
const PAGE_100: PageKey = PageKey { channel: 0, page: 0x100 };

/// A live channel's listing: pages 100, 200 and 300.
fn listing() -> Vec<Entry> {
    [0x100, 0x200, 0x300].map(|page| Entry::Page { page, title: format!("Page {page:03X}") }).to_vec()
}

/// A tab on a live channel listing [`listing`], page 100 open, with what
/// was sent so far read.
fn live(name: &str) -> (Test, u32) {
    let mut test = Test::new(&format!("redirect {name}"));
    test.core.session.open(LIVE).unwrap();
    let channel = test.core.active_channel().unwrap();
    test.core.fetched(channel, FromFetcher::Connecting(true)).unwrap();
    test.core.fetched(channel, FromFetcher::Connecting(false)).unwrap();
    test.core.fetched(channel, FromFetcher::Contents(listing())).unwrap();
    test.core.fetched(channel, FromFetcher::InitialPage(PAGE_100.page)).unwrap();
    test.core.intent(ToCore::Opened(Some(PageKey { channel, ..PAGE_100 }))).unwrap();
    test.sent();
    (test, channel)
}

fn redirected(test: &mut Test, channel: u32, page: u16, sub: u16) {
    test.core.fetched(channel, FromFetcher::PageRedirect { page, sub }).unwrap();
}

/// The pages the interface was told to open since the last look.
fn shown(test: &mut Test) -> Vec<PageKey> {
    test.sent().into_iter().filter_map(|message| if let ToInterface::Show(key) = message { Some(key) } else { None }).collect()
}

fn text(test: &Test) -> &str {
    &test.core.session.active_tab().unwrap().address().unwrap().text
}

// REQ-212 item 1, DEC-1683: the record moves the tab: the trusted core sets
// its address and tells the interface, which reports the page as opened.
#[test]
fn a_page_redirect_moves_the_tab_and_its_address() {
    let (mut test, channel) = live("page");
    redirected(&mut test, channel, 0x200, SUBCODE_ANY);
    assert_eq!(text(&test), "https://live.invalid/channel/200");
    assert_eq!(test.core.strip().address, "https://live.invalid/channel/200", "the strip says it");
    let key = PageKey { channel, page: 0x200 };
    assert_eq!(shown(&mut test), [key], "the interface is told");
    test.core.intent(ToCore::Opened(Some(key))).unwrap();
    assert_eq!(test.core.typing.selection(channel), Some((0x200, SUBCODE_ANY)), "a selection, told to the channel");
}

// DEC-1707: nothing in it asks where the keys are: here no session was ever
// welcomed and the window has lost the focus.
#[test]
fn a_redirect_moves_the_tab_wherever_the_keys_are() {
    let (mut test, channel) = live("keys");
    test.core.intent(ToCore::Focused(false)).unwrap();
    assert!(!test.core.typing.page_takes_keys());
    redirected(&mut test, channel, 0x300, SUBCODE_ANY);
    assert_eq!(text(&test), "https://live.invalid/channel/300");
}

// REQ-212 item 1: a sub-code names the sub-page, and a redirect to another
// sub-page of the open page changes the sub-page alone.
#[test]
fn a_redirect_names_a_sub_page() {
    let (mut test, channel) = live("sub-page");
    redirected(&mut test, channel, 0x200, 2);
    let key = PageKey { channel, page: 0x200 };
    assert_eq!(shown(&mut test), [key]);
    test.core.intent(ToCore::Opened(Some(key))).unwrap();
    assert_eq!(test.core.typing.selection(channel), Some((0x200, 2)));
    redirected(&mut test, channel, 0x200, 3);
    assert_eq!(shown(&mut test), [], "the page stays open");
    assert_eq!(test.core.typing.selection(channel), Some((0x200, 3)));
    redirected(&mut test, channel, 0x200, SUBCODE_ANY);
    assert_eq!(test.core.typing.selection(channel), Some((0x200, SUBCODE_ANY)), "the page itself");
}

// DEC-1683: a page the channel does not list is no redirect.
#[test]
fn a_redirect_to_a_page_not_listed_moves_nothing() {
    let (mut test, channel) = live("not listed");
    redirected(&mut test, channel, 0x400, SUBCODE_ANY);
    assert_eq!(text(&test), PAGE_100_ADDRESS);
    assert_eq!(shown(&mut test), []);
    assert_eq!(test.core.typing.selection(channel), Some((0x100, SUBCODE_ANY)));
}

// A record comes only on a push connection, whatever a fetcher says: a
// channel served as files has none to move a tab with.
#[test]
fn a_channel_served_as_files_redirects_nothing() {
    let mut test = Test::new("redirect files");
    test.core.session.open(LIVE).unwrap();
    let channel = test.core.active_channel().unwrap();
    test.core.fetched(channel, FromFetcher::Contents(listing())).unwrap();
    test.sent();
    redirected(&mut test, channel, 0x200, SUBCODE_ANY);
    assert_eq!(text(&test), LIVE);
    assert_eq!(shown(&mut test), []);
}

// REQ-1686 item 8's rule for a page redirect: the tab behind others moves,
// and its address opens the page when it comes to the front.
#[test]
fn a_tab_behind_others_moves() {
    let (mut test, channel) = live("behind");
    test.core.session.open("https://other.invalid/").unwrap();
    let _ = test.sent();
    redirected(&mut test, channel, 0x200, SUBCODE_ANY);
    assert_eq!(test.core.session.tabs[0].address().unwrap().text, "https://live.invalid/channel/200");
    assert_eq!(test.core.typing.selection(channel), Some((0x200, SUBCODE_ANY)), "the channel is told");
    assert_eq!(shown(&mut test), [], "not its window's page");
}

// TODO-1656, `INPUT.md` section 7.1: a cursor record moves nothing, whatever
// page it names and however late it comes. Only the REDIRECT record does.
#[test]
fn a_cursor_record_never_moves_a_viewer() {
    let (mut test, channel) = live("cursor");
    welcomed(&mut test, channel);
    test.core.intent(ToCore::Opened(Some(PageKey { channel, page: 0x200 }))).unwrap();
    test.sent();
    let [high, low] = 0x100_u16.to_be_bytes();
    let left = FromDecoder::Cursor { body: [high, low, 0, 0, neoteletext_input::cursor::NO_CURSOR, 0, 0, 0] };
    test.core.decoded(channel, left).unwrap();
    assert_eq!(shown(&mut test), []);
    assert_eq!(test.core.typing.selection(channel), Some((0x200, SUBCODE_ANY)));
    redirected(&mut test, channel, 0x100, SUBCODE_ANY);
    assert_eq!(shown(&mut test), [PageKey { channel, page: 0x100 }], "the record does");
}

/// A tab on a live channel, with a stand-in broker for the channels it goes to.
fn live_with_broker() -> (Core, StandIn, u32) {
    let (mut core, stand_in) = core_on(LIVE);
    let channel = channel(&core);
    core.fetched(channel, FromFetcher::Connecting(true)).unwrap();
    core.fetched(channel, FromFetcher::Connecting(false)).unwrap();
    (core, stand_in, channel)
}

fn program_redirect(core: &mut Core, location: &str) {
    let live = channel(core);
    core.fetched(live, FromFetcher::Connecting(false)).unwrap();
    core.fetched(live, FromFetcher::ProgramRedirect(location.into())).unwrap();
}

// REQ-1688 item 4, REQ-1686 item 1: a program's redirect to another channel
// shows the notice for a second, then opens it as a channel of its own,
// recorded as started by the page. It can come long after the channel opened,
// as a server's redirect cannot (DEC-1696).
#[test]
fn a_programs_redirect_shows_the_notice_then_opens_the_channel() {
    let (mut core, _stand_in, old) = live_with_broker();
    core.fetched(old, FromFetcher::InitialPage(0x100)).unwrap();
    program_redirect(&mut core, DESTINATION);
    assert_eq!(core.strip().redirecting, DESTINATION);
    assert_eq!(address(&core), "https://live.invalid/channel/100", "the address waits");
    elapse(&mut core);
    assert_eq!(address(&core), DESTINATION);
    assert_ne!(channel(&core), old, "a channel of its own");
    let tab = core.session.active_tab().unwrap();
    assert!(tab.history[tab.position].by_page, "started by the page");
}

// REQ-1688 item 3: the address is hostile text, and leads only to the web.
#[test]
fn a_programs_redirect_leads_only_to_the_web() {
    let (mut core, _stand_in, old) = live_with_broker();
    for location in
        ["file:///etc/passwd", "neoteletext:home", "web:https://example.invalid/", "ftp://example.invalid/", "https://user@example.invalid/", "/etc/passwd"]
    {
        program_redirect(&mut core, location);
        assert_eq!(core.strip().redirecting, "", "{location}");
        elapse(&mut core);
        assert_eq!((address(&core), channel(&core)), (LIVE, old), "{location} leaves the tab");
    }
    assert_eq!(core.session.active_tab().unwrap().history.len(), 1);
}

// REQ-1686 item 1: an address of the channel itself stays in it, with no
// notice, and is a page redirect.
#[test]
fn a_programs_redirect_to_its_own_channel_shows_no_notice() {
    let (mut test, channel) = live("own");
    test.core.fetched(channel, FromFetcher::ProgramRedirect("https://live.invalid/channel/300".into())).unwrap();
    assert_eq!(test.core.strip().redirecting, "");
    assert_eq!(text(&test), "https://live.invalid/channel/300");
    assert_eq!(shown(&mut test), [PageKey { channel, page: 0x300 }]);
}

// REQ-1688 item 6: two channels that redirect to each other stop at the
// bound, with no refusal for coming round, and the tab stays where it is.
#[test]
fn two_channels_redirecting_to_each_other_stop_at_the_bound() {
    let (mut core, _stand_in, _) = live_with_broker();
    let (first, second) = (LIVE, DESTINATION);
    for hop_number in 1..=MAX_REDIRECTS {
        let location = if hop_number % 2 == 1 { second } else { first };
        program_redirect(&mut core, location);
        assert_eq!(core.strip().redirecting, location, "hop {hop_number} has its own notice");
        elapse(&mut core);
        assert_eq!(address(&core), location, "hop {hop_number} is followed");
    }
    let last = channel(&core);
    let here = address(&core).to_owned();
    program_redirect(&mut core, if here == first { second } else { first });
    assert_eq!(core.strip().redirecting, "", "past the bound");
    elapse(&mut core);
    assert_eq!((address(&core), channel(&core)), (here.as_str(), last), "dropped, and the tab stays");
}

// REQ-1688 item 6: a viewer's input starts the count again.
#[test]
fn the_viewers_input_starts_the_count_again() {
    let (mut core, _stand_in, _) = live_with_broker();
    for hop_number in 1..=MAX_REDIRECTS {
        program_redirect(&mut core, &format!("https://hop{hop_number}.invalid/"));
        elapse(&mut core);
    }
    let last = channel(&core);
    core.session.viewer_input(last);
    program_redirect(&mut core, "https://onemore.invalid/");
    assert_eq!(core.strip().redirecting, "https://onemore.invalid/");
}

// REQ-1688's hostile case: the longest address a record can carry is read,
// shown and followed. One past it never gets here: the engine's reader
// refuses the record whole, and `a_record_past_its_bound_is_refused` shows it.
#[test]
fn the_longest_address_a_record_carries_is_followed() {
    use neoteletext_web::records::redirect::MAX_ADDRESS;
    // An address names a page to take up the last of the bound: one with no
    // page and no room for one is refused (REQ-1690 item 10).
    let head = "https://long.invalid/302/0002?q=";
    let location = format!("{head}{}", "a".repeat(MAX_ADDRESS - head.len()));
    assert_eq!(location.len(), MAX_ADDRESS);
    let (mut core, _stand_in, _) = live_with_broker();
    program_redirect(&mut core, &location);
    assert_eq!(core.strip().redirecting, location);
    elapse(&mut core);
    assert_eq!(address(&core), location);
}

/// `channel`'s session with the channel started, so that input goes.
fn welcomed(test: &mut Test, channel: u32) {
    test.core.typing.input(channel, true, false, 0);
    let mut body = [0_u8; neoteletext_input::control::MAX_BODY];
    let welcome = neoteletext_input::control::Control::Welcome { version: neoteletext_input::VERSION, caps: neoteletext_input::caps::SELECT_CAP, resume: None };
    let length = neoteletext_input::control::write_body(&welcome, &mut body).unwrap();
    test.core.typing.control(channel, &body[..length], 0);
}

/// The open page's first frame is drawn.
fn draw(test: &mut Test, channel: u32, page: u16) {
    let step = SubPageStep { count: 1, shown: 0, place: Some(0), previous: None, next: None };
    test.core.decoded(channel, FromDecoder::SubPages { page, step }).unwrap();
}

/// A key is pressed on the open page.
fn press(test: &mut Test, channel: u32, page: u16) {
    let intent = Intent::Key { action: Action::Press, key: KeyCode::Character('a'), mods: 0, text: Some("a".into()), base: None };
    test.core.intent(ToCore::Input(InputIntent { key: PageKey { channel, page }, intent })).unwrap();
}

fn entries(test: &Test) -> Vec<(String, bool)> {
    test.core.session.active_tab().unwrap().history.iter().map(|visit| (visit.address.text.clone(), visit.passed_over)).collect()
}

const PAGE_100_ADDRESS: &str = "https://live.invalid/channel/100";
const PAGE_200_ADDRESS: &str = "https://live.invalid/channel/200";
const PAGE_300_ADDRESS: &str = "https://live.invalid/channel/300";

/// A tab on a live channel on page 100, named by its address and drawn.
fn on_page_100(name: &str) -> (Test, u32) {
    let (mut test, channel) = live(name);
    welcomed(&mut test, channel);
    draw(&mut test, channel, 0x100);
    (test, channel)
}

// DEC-1684 item 2: with the viewer's input on the page, an ordinary entry:
// Back returns to the page acted on.
#[test]
fn a_redirect_after_input_adds_an_entry() {
    let (mut test, channel) = on_page_100("after input");
    press(&mut test, channel, 0x100);
    redirected(&mut test, channel, 0x200, SUBCODE_ANY);
    assert_eq!(entries(&test).last().unwrap(), &(PAGE_200_ADDRESS.to_owned(), false));
    assert!(entries(&test).iter().all(|(_, passed_over)| !passed_over));
    assert_eq!(test.core.session.go(false), Went::Page(Some(0x100)), "Back returns to it");
}

// DEC-1684 item 3: a page that moves on arrival was never seen, and leaves
// no entry to be trapped by.
#[test]
fn a_redirect_before_the_page_drew_replaces_its_entry() {
    let (mut test, channel) = live("before drawn");
    let before = entries(&test).len();
    redirected(&mut test, channel, 0x200, SUBCODE_ANY);
    assert_eq!(entries(&test), [(PAGE_200_ADDRESS.to_owned(), false)]);
    assert_eq!(entries(&test).len(), before);
}

// DEC-1684 item 4: read for a while, then moved with no act of the viewer's:
// an entry, and the buttons pass over the page left, which stays in the list.
#[test]
fn a_redirect_after_the_page_drew_marks_the_page_left() {
    let (mut test, channel) = on_page_100("after drawn");
    let before = entries(&test).len();
    redirected(&mut test, channel, 0x200, SUBCODE_ANY);
    let list = entries(&test);
    assert_eq!(list.len(), before + 1);
    assert_eq!(list[before - 1], (PAGE_100_ADDRESS.to_owned(), true), "marked");
    assert_eq!(list[before], (PAGE_200_ADDRESS.to_owned(), false));
    assert!(test.core.session.tab_list(false).iter().any(|(_, visit)| visit.address.text == PAGE_100_ADDRESS), "in the list");
    let went = test.core.session.go(false);
    assert_ne!(test.core.session.active_tab().unwrap().address().unwrap().text, PAGE_100_ADDRESS, "the button passed it: {went:?}");
}

// DEC-1684 item 5: the viewer's input clears the mark, and coming back does
// not; item 7: it counts on the page it was given to, and a new selection
// starts afresh.
#[test]
fn input_clears_the_mark_and_counts_only_on_its_page() {
    let (mut test, channel) = on_page_100("input clears");
    redirected(&mut test, channel, 0x200, SUBCODE_ANY);
    assert_eq!(test.core.session.go_to(false, 1), Went::Page(Some(0x100)), "from the list");
    test.core.intent(ToCore::Opened(Some(PageKey { channel, page: 0x100 }))).unwrap();
    assert!(test.core.session.active_tab().unwrap().history[test.core.session.active_tab().unwrap().position].passed_over, "coming back does not clear it");
    draw(&mut test, channel, 0x100);
    press(&mut test, channel, 0x100);
    assert!(!test.core.session.active_tab().unwrap().history[test.core.session.active_tab().unwrap().position].passed_over, "input does");
    // Input moved it on; the next page has had none.
    redirected(&mut test, channel, 0x200, SUBCODE_ANY);
    test.core.intent(ToCore::Opened(Some(PageKey { channel, page: 0x200 }))).unwrap();
    draw(&mut test, channel, 0x200);
    redirected(&mut test, channel, 0x300, SUBCODE_ANY);
    let list = entries(&test);
    assert_eq!(list[list.len() - 2], (PAGE_200_ADDRESS.to_owned(), true), "one input allows one entry");
    assert_eq!(list[list.len() - 1], (PAGE_300_ADDRESS.to_owned(), false));
}

// DEC-1684 item 8: a channel redirecting on a timer pushes out only its own
// entries: what the viewer chose stays, and Back returns to it.
#[test]
fn a_channel_redirecting_on_a_timer_keeps_the_viewers_entries() {
    let (mut test, channel) = on_page_100("timer");
    press(&mut test, channel, 0x100);
    let viewers = entries(&test);
    for round in 0..2 * MAX_TAB_ENTRIES {
        let page = if round % 2 == 0 { 0x200 } else { 0x300 };
        // The viewer's entries are its own, and this page gets no input.
        redirected(&mut test, channel, page, SUBCODE_ANY);
        test.core.intent(ToCore::Opened(Some(PageKey { channel, page }))).unwrap();
        draw(&mut test, channel, page);
    }
    let list = entries(&test);
    assert_eq!(list.len(), MAX_TAB_ENTRIES);
    assert!(viewers.iter().all(|(address, _)| list.iter().any(|(kept, _)| kept == address)), "{list:?}");
    assert_eq!(test.core.session.go(false), Went::Page(Some(0x100)), "Back is the viewer's page, past all of them");
}

// DEC-1684 items 2 to 4 hold for a program's redirect to another channel.
#[test]
fn a_redirect_to_another_channel_follows_the_same_rules() {
    let hop_after = |prepare: fn(&mut Core, u32)| {
        let (mut core, _stand_in, old) = live_with_broker();
        prepare(&mut core, old);
        program_redirect(&mut core, DESTINATION);
        elapse(&mut core);
        let tab = core.session.active_tab().unwrap();
        tab.history.iter().map(|visit| (visit.address.text.clone(), visit.passed_over)).collect::<Vec<_>>()
    };
    let there = (DESTINATION.to_owned(), false);
    assert_eq!(hop_after(|_, _| {}), core::slice::from_ref(&there), "not drawn: replaced");
    assert_eq!(hop_after(|core, old| core.session.page_drawn(old)), [(LIVE.to_owned(), true), there.clone()], "drawn: marked");
    let acted: fn(&mut Core, u32) = |core, old| {
        core.session.page_drawn(old);
        core.session.viewer_input(old);
    };
    assert_eq!(hop_after(acted), [(LIVE.to_owned(), false), there], "input: an entry");
}
