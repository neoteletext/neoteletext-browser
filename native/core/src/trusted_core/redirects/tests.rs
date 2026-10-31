//! REQ-701 and REQ-1686: a redirect that leaves a channel is followed as a
//! web browser follows one, after a second's notice, against the stand-in
//! broker the new channel's helpers need.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]

use std::time::{Duration, Instant};

use confine::broker::Program;
use confine::broker::stand_in::StandIn;
use interface_core::canvas::PageKey;
use neoteletext_core::limits::SUBCODE_ANY;
use protocol::decoder::FromDecoder;
use protocol::fetcher::{Entry, FromFetcher};
use protocol::{ToCore, ToInterface};

use super::super::Core;
use super::super::test_support::{Test, core_on};
use super::REDIRECT_NOTICE;
use crate::session::MAX_REDIRECTS;

const BEFORE: &str = "https://before.invalid/teletext/";
const REDIRECTING: &str = "https://moved.invalid/teletext/";
const DESTINATION: &str = "https://new.invalid/teletext/";
const ONE_MILLISECOND: Duration = Duration::from_millis(1);

/// A tab that was on [`BEFORE`] and went to [`REDIRECTING`], whose fetcher
/// is starting.
fn opening() -> (Core, StandIn) {
    let (mut core, stand_in) = core_on(BEFORE);
    core.intent(ToCore::Navigate(REDIRECTING.into())).unwrap();
    (core, stand_in)
}

fn address(core: &Core) -> &str {
    &core.session.active_tab().unwrap().address().unwrap().text
}

fn channel(core: &Core) -> u32 {
    core.session.active_tab().unwrap().channel.id
}

fn redirect(core: &mut Core, location: &str) {
    core.fetched(channel(core), FromFetcher::Redirected(location.into())).unwrap();
}

/// The notice's second ends.
fn elapse(core: &mut Core) {
    let due = core.redirect_due_at().unwrap_or_else(Instant::now);
    core.finish_redirects(due).unwrap();
}

/// A redirect followed: said, and its second over.
fn hop(core: &mut Core, location: &str) {
    redirect(core, location);
    elapse(core);
}

/// Whether the strip says `words`, as a channel that failed to open does.
fn says(core: &Core, words: &str) -> bool {
    core.strip().notice.contains(words)
}

// REQ-1686 items 2, 4, 5 and 6: the trusted core names the destination, the
// address field keeps the old address, and nothing moves until the second
// is up, for exactly the named time; then the notice goes.
#[test]
fn a_redirect_shows_its_notice_for_a_second_then_moves() {
    let (mut core, _stand_in) = opening();
    let old = channel(&core);
    let said = Instant::now();
    redirect(&mut core, DESTINATION);
    assert_eq!(core.strip().redirecting, DESTINATION, "the notice names it");
    assert_eq!(core.strip().address, REDIRECTING, "the address field keeps the old address");
    let due = core.redirect_due_at().unwrap();
    assert!(due >= said + REDIRECT_NOTICE && due <= Instant::now() + REDIRECT_NOTICE, "exactly the notice's time");
    assert!(core.next_wake().unwrap() <= due, "the core wakes for it");
    core.finish_redirects(due.checked_sub(ONE_MILLISECOND).unwrap()).unwrap();
    assert_eq!((address(&core), channel(&core)), (REDIRECTING, old), "not before the second is up");
    core.finish_redirects(due).unwrap();
    assert_eq!(address(&core), DESTINATION);
    assert_eq!(core.strip().redirecting, "", "the notice has gone");
    assert_eq!(core.redirect_due_at(), None);
}

// REQ-701 items 1 and 3: no question, the address shows the destination, the
// destination has its own channel and fetcher, and the visit is the page's.
#[test]
fn a_redirect_moves_the_tab_with_no_question() {
    let (mut core, stand_in) = opening();
    let old = channel(&core);
    hop(&mut core, DESTINATION);
    assert_eq!(address(&core), DESTINATION);
    assert_eq!(core.strip().address, DESTINATION, "the address field shows the destination");
    assert_ne!(channel(&core), old, "a channel of its own");
    assert!(core.session.active_tab().unwrap().history.last().unwrap().by_page, "recorded as the page's");
    let fetchers = stand_in.started().into_iter().filter(|helper| helper.program == Program::Fetcher).count();
    assert_eq!(fetchers, 2, "the redirecting address's, then the destination's");
    assert!(core.fetchers.contains_key(&channel(&core)), "the destination's fetcher");
    assert!(!core.fetchers.contains_key(&old), "the redirecting channel's is gone");
}

// REQ-701 item 3: Back skips the address that redirected.
#[test]
fn back_returns_to_the_page_before_the_redirecting_address() {
    let (mut core, _stand_in) = opening();
    hop(&mut core, DESTINATION);
    assert_eq!(address(&core), DESTINATION);
    assert_eq!(core.session.active_tab().unwrap().history.len(), 2, "the destination replaced the address that redirected");
    core.intent(ToCore::Back).unwrap();
    assert_eq!(address(&core), BEFORE);
}

// REQ-1686 item 7: Back during the second replaces the redirect.
#[test]
fn back_during_the_second_drops_the_redirect() {
    let (mut core, _stand_in) = opening();
    redirect(&mut core, DESTINATION);
    assert_eq!(address(&core), REDIRECTING, "within its second");
    core.intent(ToCore::Back).unwrap();
    assert_eq!(address(&core), BEFORE);
    assert_eq!(core.strip().redirecting, "", "the notice went with it");
    assert_eq!(core.redirect_due_at(), None);
    core.finish_redirects(Instant::now() + 2 * REDIRECT_NOTICE).unwrap();
    assert_eq!(address(&core), BEFORE, "the tab stays where Back took it");
}

// REQ-1686 item 7: so does any other navigation of the viewer's own.
#[test]
fn the_viewers_own_navigation_drops_the_redirect() {
    for (intent, stays) in [
        (ToCore::Navigate("https://typed.invalid/".into()), "https://typed.invalid/"),
        (ToCore::Reload, REDIRECTING),
        (ToCore::Navigate(format!("{REDIRECTING}#302")), "https://moved.invalid/teletext/#302"),
    ] {
        let (mut core, _stand_in) = opening();
        redirect(&mut core, DESTINATION);
        core.intent(intent).unwrap();
        assert_eq!(core.redirect_due_at(), None, "{stays}");
        assert_eq!(core.strip().redirecting, "", "{stays}");
        assert_eq!(address(&core), stays);
    }
}

// REQ-1686 item 7: a tab closed takes its redirect.
#[test]
fn closing_the_tab_drops_the_redirect() {
    let (mut core, _stand_in) = opening();
    core.session.open("https://other.invalid/").unwrap();
    core.send_all().unwrap();
    let behind = core.session.tabs[0].channel.id;
    core.fetched(behind, FromFetcher::Redirected(DESTINATION.into())).unwrap();
    assert!(core.redirect_due_at().is_some());
    core.intent(ToCore::CloseTab(0)).unwrap();
    assert_eq!(core.redirect_due_at(), None);
}

// REQ-1686 item 8: another tab, even of the same channel, is not touched.
#[test]
fn a_second_tab_of_the_channel_keeps_its_page() {
    let (mut core, _stand_in) = opening();
    let first = channel(&core);
    core.session.open(REDIRECTING).unwrap();
    core.send_all().unwrap();
    let second = channel(&core);
    core.fetched(first, FromFetcher::Redirected(DESTINATION.into())).unwrap();
    assert_eq!(core.strip().redirecting, "", "the tab in front shows no notice");
    assert_eq!(core.session.tabs[0].address().unwrap().text, REDIRECTING, "within its second");
    elapse(&mut core);
    assert_eq!((address(&core), channel(&core)), (REDIRECTING, second), "the tab in front stays");
    assert_eq!(core.session.tabs[0].address().unwrap().text, DESTINATION, "the tab behind ran its own second");
}

// REQ-1686 item 8: switching to a tab within its second shows its notice,
// and switching away puts it away.
#[test]
fn the_notice_is_the_tabs_own() {
    let (mut core, _stand_in) = opening();
    let first = channel(&core);
    core.session.open("https://other.invalid/").unwrap();
    core.send_all().unwrap();
    core.fetched(first, FromFetcher::Redirected(DESTINATION.into())).unwrap();
    core.intent(ToCore::SelectTab(0)).unwrap();
    assert_eq!(core.strip().redirecting, DESTINATION);
    core.intent(ToCore::SelectTab(1)).unwrap();
    assert_eq!(core.strip().redirecting, "");
}

// REQ-701 item 2: a redirect leads only to an address the rules accept.
#[test]
fn a_refused_destination_is_refused() {
    for location in
        ["file:///etc/passwd", "/etc/passwd", "neoteletext:home", "ftp://new.invalid/", "https://user@new.invalid/", "new.invalid", "javascript:alert(1)", ""]
    {
        let (mut core, _stand_in) = opening();
        let old = channel(&core);
        redirect(&mut core, location);
        assert_eq!(core.strip().redirecting, "", "{location}: no notice");
        elapse(&mut core);
        assert_eq!((address(&core), channel(&core)), (REDIRECTING, old), "{location}");
        assert!(says(&core, "could not open"), "{location}");
    }
}

// REQ-701 item 4, REQ-1686 item 9: twenty are followed, each with its own
// notice, and the twenty-first is refused, as the Fetch standard's limit is.
#[test]
fn a_chain_past_the_bound_fails() {
    let (mut core, _stand_in) = opening();
    for hop_number in 1..=MAX_REDIRECTS {
        let location = format!("https://hop{hop_number}.invalid/");
        redirect(&mut core, &location);
        assert_eq!(core.strip().redirecting, location, "hop {hop_number} has its own notice");
        elapse(&mut core);
        assert_eq!(address(&core), location, "hop {hop_number} is followed");
    }
    let last = channel(&core);
    redirect(&mut core, "https://onetoomany.invalid/");
    assert_eq!(core.strip().redirecting, "", "no notice");
    elapse(&mut core);
    assert_eq!(channel(&core), last, "refused");
    assert!(says(&core, "too many redirects"));
    assert_eq!(core.session.active_tab().unwrap().history.len(), 2);
}

// REQ-701 item 4: a chain that comes round again stops at once.
#[test]
fn a_loop_fails() {
    let (mut core, _stand_in) = opening();
    hop(&mut core, DESTINATION);
    let last = channel(&core);
    redirect(&mut core, REDIRECTING);
    elapse(&mut core);
    assert_eq!((address(&core), channel(&core)), (DESTINATION, last), "refused");
    assert!(says(&core, "redirects in a loop"));
}

// REQ-701 item 4: the chain ends with it; a new navigation counts afresh.
#[test]
fn the_chain_is_the_navigations() {
    let (mut core, _stand_in) = opening();
    hop(&mut core, DESTINATION);
    core.intent(ToCore::Navigate(REDIRECTING.into())).unwrap();
    hop(&mut core, DESTINATION);
    assert_eq!(address(&core), DESTINATION, "not a loop: the viewer went there");
}

// A fetcher says it again and again: its second is not put off.
#[test]
fn saying_it_again_does_not_put_the_second_off() {
    let (mut core, _stand_in) = opening();
    redirect(&mut core, DESTINATION);
    let due = core.redirect_due_at().unwrap();
    redirect(&mut core, "https://later.invalid/");
    assert_eq!(core.redirect_due_at(), Some(due));
    assert_eq!(core.strip().redirecting, DESTINATION);
}

// REQ-701 item 5: https to http is followed, and the strip says so.
#[test]
fn a_downgrade_is_followed_and_shown_not_secure() {
    let (mut core, _stand_in) = opening();
    assert!(core.strip().encrypted);
    hop(&mut core, "http://new.invalid/teletext/");
    assert_eq!(address(&core), "http://new.invalid/teletext/");
    assert!(!core.strip().encrypted);
}

// RFC 9110 section 10.2.2, as REQ-505 keeps a fragment off the wire: the
// page the viewer asked for goes with a redirect that names none.
#[test]
fn a_fragment_goes_with_a_redirect_that_names_none() {
    let (mut core, _stand_in) = core_on(BEFORE);
    core.intent(ToCore::Navigate(format!("{REDIRECTING}#302"))).unwrap();
    redirect(&mut core, DESTINATION);
    assert_eq!(core.strip().redirecting, format!("{DESTINATION}#302"), "the complete address");
    elapse(&mut core);
    assert_eq!(address(&core), format!("{DESTINATION}#302"));
    core.intent(ToCore::Navigate(format!("{REDIRECTING}#302"))).unwrap();
    hop(&mut core, &format!("{DESTINATION}#401"));
    assert_eq!(address(&core), format!("{DESTINATION}#401"));
}

// REQ-701: a page fetched once the channel is open is no navigation, and a
// fetcher, which is untrusted, cannot move its tab by saying one is.
#[test]
fn an_open_channel_is_not_moved() {
    let (mut core, _stand_in) = opening();
    let open = channel(&core);
    core.fetched(open, FromFetcher::InitialPage(0x100)).unwrap();
    hop(&mut core, DESTINATION);
    assert_eq!((address(&core), channel(&core)), (REDIRECTING, open));
    assert_eq!(core.strip().redirecting, "");
}

// REQ-701: a message from a channel no tab holds moves nothing.
#[test]
fn a_gone_channel_moves_nothing() {
    let (mut core, _stand_in) = opening();
    let (shown, only) = (address(&core).to_owned(), channel(&core));
    core.fetched(only + 1_000, FromFetcher::Redirected(DESTINATION.into())).unwrap();
    assert_eq!(core.redirect_due_at(), None);
    assert_eq!((address(&core), channel(&core)), (shown.as_str(), only));
}

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
    assert_eq!(text(&test), "https://live.invalid/channel/#200");
    assert_eq!(test.core.strip().address, "https://live.invalid/channel/#200", "the strip says it");
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
    assert_eq!(text(&test), "https://live.invalid/channel/#300");
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
    assert_eq!(text(&test), LIVE);
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
    assert_eq!(test.core.session.tabs[0].address().unwrap().text, "https://live.invalid/channel/#200");
    assert_eq!(test.core.typing.selection(channel), Some((0x200, SUBCODE_ANY)), "the channel is told");
    assert_eq!(shown(&mut test), [], "not its window's page");
}

// TODO-1656, `INPUT.md` section 7.1: a cursor record moves nothing, whatever
// page it names and however late it comes. Only the REDIRECT record does.
#[test]
fn a_cursor_record_never_moves_a_viewer() {
    let (mut test, channel) = live("cursor");
    test.core.typing.input(channel, true, false, 0);
    let mut body = [0_u8; neoteletext_input::control::MAX_BODY];
    let length = neoteletext_input::control::write_body(
        &neoteletext_input::control::Control::Welcome { version: neoteletext_input::VERSION, caps: neoteletext_input::caps::SELECT_CAP, resume: None },
        &mut body,
    )
    .unwrap();
    test.core.typing.control(channel, &body[..length], 0);
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
    assert_eq!(address(&core), LIVE, "the address waits");
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
    test.core.fetched(channel, FromFetcher::ProgramRedirect("https://live.invalid/channel/#300".into())).unwrap();
    assert_eq!(test.core.strip().redirecting, "");
    assert_eq!(text(&test), "https://live.invalid/channel/#300");
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
    let head = "https://long.invalid/";
    let location = format!("{head}{}", "a".repeat(MAX_ADDRESS - head.len()));
    assert_eq!(location.len(), MAX_ADDRESS);
    let (mut core, _stand_in, _) = live_with_broker();
    program_redirect(&mut core, &location);
    assert_eq!(core.strip().redirecting, location);
    elapse(&mut core);
    assert_eq!(address(&core), location);
}
