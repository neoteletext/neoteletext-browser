//! REQ-1690 and REQ-1693: a tab's address names what it has selected, by
//! whatever path the selection changed.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]

use interface_core::canvas::PageKey;
use interface_core::controls::PageControl;
#[cfg(feature = "interaction")]
use neoteletext_core::limits::SUBCODE_ANY;
use protocol::decoder::FromDecoder;
use protocol::fetcher::{Entry, FromFetcher};
use protocol::library::HistoryRequest;
use protocol::subpages::{SubPageRows, SubPageStep};
use protocol::{ToCore, ToInterface};

use super::test_support::Test;
use crate::session::Went;

const CHANNEL: &str = "https://live.invalid/channel/";

fn at(fragment: &str) -> String {
    format!("{CHANNEL}{fragment}")
}

/// A tab on a live channel listing pages 100, 200 and 300, and sub-pages 1,
/// 2 and 3 of page 200, that has arrived at page 100.
fn arrived(name: &str) -> (Test, u32) {
    let mut test = Test::new(&format!("address {name}"));
    test.core.session.open(CHANNEL).unwrap();
    let channel = test.core.active_channel().unwrap();
    test.core.fetched(channel, FromFetcher::Connecting(true)).unwrap();
    test.core.fetched(channel, FromFetcher::Connecting(false)).unwrap();
    let pages = [0x100, 0x200, 0x300].map(|page| Entry::Page { page, title: format!("Page {page:03X}") }).to_vec();
    test.core.fetched(channel, FromFetcher::Contents(pages)).unwrap();
    let rows = (1_u16..=3).map(|subcode| (subcode, String::new())).collect();
    test.core.fetched(channel, FromFetcher::SubPages { page: 0x200, version: "1".into(), part: SubPageRows { count: 3, first: 0, rows } }).unwrap();
    test.core.fetched(channel, FromFetcher::InitialPage(0x100)).unwrap();
    test.core.intent(ToCore::Opened(Some(PageKey { channel, page: 0x100 }))).unwrap();
    test.sent();
    (test, channel)
}

fn open(test: &mut Test, channel: u32, page: u16) {
    test.core.intent(ToCore::Opened(Some(PageKey { channel, page }))).unwrap();
}

fn shown(test: &Test) -> String {
    test.core.session.active_tab().unwrap().address().unwrap().text.clone()
}

fn entries(test: &Test) -> usize {
    test.core.session.active_tab().unwrap().history.len()
}

// REQ-1693 item 3: arriving at the bare address names the initial page, in
// the same entry, and the page opens.
#[test]
fn arriving_at_a_channel_names_its_initial_page() {
    let mut test = Test::new("address arriving");
    test.core.session.open(CHANNEL).unwrap();
    let channel = test.core.active_channel().unwrap();
    test.sent();
    test.core.fetched(channel, FromFetcher::InitialPage(0x100)).unwrap();
    assert_eq!(shown(&test), at("100"));
    assert_eq!(entries(&test), 1, "no Back entry");
    let sent = test.sent();
    assert!(sent.contains(&ToInterface::Show(PageKey { channel, page: 0x100 })), "{sent:?}");
    #[cfg(feature = "interaction")]
    assert_eq!(test.core.typing.selection(channel), Some((0x100, SUBCODE_ANY)), "and it is selected");
}

// REQ-1693 item 3: an address that names a place keeps it.
#[test]
fn an_address_that_names_a_place_arrives_there() {
    for fragment in ["302", "wall"] {
        let mut test = Test::new(&format!("address place {fragment}"));
        test.core.session.open(&at(fragment)).unwrap();
        let channel = test.core.active_channel().unwrap();
        test.core.fetched(channel, FromFetcher::InitialPage(0x100)).unwrap();
        assert_eq!(shown(&test), at(fragment));
    }
}

// REQ-1690 items 1 and 3: every way of choosing a page reaches the core as
// one report; the address follows, with a Back entry, and Back returns.
#[test]
fn the_address_follows_a_page_the_viewer_chose() {
    let (mut test, channel) = arrived("chosen");
    open(&mut test, channel, 0x200);
    assert_eq!(shown(&test), at("200"));
    assert_eq!(test.core.strip().address, at("200"), "the strip says it");
    assert_eq!(entries(&test), 2);
    open(&mut test, channel, 0x200);
    assert_eq!(entries(&test), 2, "once");
    let went = test.core.session.go(false);
    assert_eq!(went, Went::Page(Some(0x100)));
    assert_eq!(shown(&test), at("100"));
}

// REQ-1690: a followed link and a redirect moved the address already; what
// the interface reports after them adds nothing.
#[cfg(feature = "links")]
#[test]
fn a_page_the_core_opened_counts_once() {
    let (mut test, channel) = arrived("once");
    test.core.session.show_page(0x200).unwrap();
    open(&mut test, channel, 0x200);
    assert_eq!(entries(&test), 2);
}

// REQ-1690 item 3: a page chosen is a visit when it draws, not a page the
// viewer never reached.
#[test]
fn a_page_chosen_is_a_visit() {
    let (mut test, channel) = arrived("visit");
    open(&mut test, channel, 0x200);
    test.core.history_request(HistoryRequest::Panel(true)).unwrap();
    let visits = test.last_history();
    assert!(visits.rows.iter().any(|row| row.address == at("200")), "{:?}", visits.rows);
}

// REQ-1690 item 4: a step names the sub-page in the entry, with no Back
// entry; a page's first sub-page is the page.
#[test]
fn a_step_names_the_sub_page_in_the_same_entry() {
    let (mut test, channel) = arrived("step");
    open(&mut test, channel, 0x200);
    let step = SubPageStep { count: 1, shown: 1, place: Some(0), previous: None, next: None };
    test.core.decoded(channel, FromDecoder::SubPages { page: 0x200, step }).unwrap();
    assert_eq!(shown(&test), at("200"), "the first sub-page, adopted, is the page");
    let before = entries(&test);
    test.core.intent(ToCore::PageControl(PageControl::SubPage(true))).unwrap();
    assert_eq!(shown(&test), at("200/0002"));
    test.core.intent(ToCore::PageControl(PageControl::SubPage(true))).unwrap();
    assert_eq!(shown(&test), at("200/0003"));
    assert_eq!(entries(&test), before, "no Back entry");
    test.core.intent(ToCore::PageControl(PageControl::SubPageOf { page: 0x200, subcode: 1 })).unwrap();
    assert_eq!(shown(&test), at("200/0001"), "chosen from the list");
}

// REQ-1690 item 5: leaving a page for the wall names the wall in the same
// entry, a tile opened from it adds one, and Back to it shows the wall.
#[test]
fn the_wall_has_an_address() {
    let (mut test, channel) = arrived("wall");
    test.core.intent(ToCore::Opened(None)).unwrap();
    assert_eq!(shown(&test), at("wall"));
    assert_eq!(entries(&test), 1);
    open(&mut test, channel, 0x300);
    assert_eq!(shown(&test), at("300"));
    assert_eq!(entries(&test), 2);
    test.sent();
    test.core.intent(ToCore::Back).unwrap();
    assert_eq!(shown(&test), at("wall"));
    let sent = test.sent();
    assert!(sent.contains(&ToInterface::ShowWall(channel)), "{sent:?}");
}

// A tab's first report, with no page yet, is no leaving of a page.
#[test]
fn no_page_yet_is_not_the_wall() {
    let mut test = Test::new("address not wall");
    test.core.session.open(CHANNEL).unwrap();
    test.core.intent(ToCore::Opened(None)).unwrap();
    assert_eq!(shown(&test), CHANNEL);
}

// REQ-1690 item 9: a file's page changes its address.
#[cfg(unix)]
#[test]
fn a_files_page_names_its_address() {
    let mut test = Test::new("address file");
    test.core.session.open("/tmp/news.tti").unwrap();
    let channel = test.core.active_channel().unwrap();
    open(&mut test, channel, 0x200);
    assert_eq!(shown(&test), "file:///tmp/news.tti#200");
}

// REQ-1693 item 9: the first page a file shows is where a tab at its bare
// address arrives, in the same entry; a later page is a page chosen.
#[cfg(unix)]
#[test]
fn arriving_at_a_file_adds_no_back_entry() {
    let mut test = Test::new("address file arrival");
    test.core.session.open("/tmp/news.tti").unwrap();
    let channel = test.core.active_channel().unwrap();
    open(&mut test, channel, 0x100);
    assert_eq!((shown(&test).as_str(), entries(&test)), ("file:///tmp/news.tti#100", 1));
    open(&mut test, channel, 0x200);
    assert_eq!((shown(&test).as_str(), entries(&test)), ("file:///tmp/news.tti#200", 2));
}

// REQ-1693 item 1: the contents have an address. A viewer's page leaves it,
// and Back returns to it.
#[test]
fn the_contents_have_an_address() {
    let mut test = Test::new("address contents");
    test.core.session.open(&at("contents")).unwrap();
    let channel = test.core.active_channel().unwrap();
    test.sent();
    test.core.fetched(channel, FromFetcher::InitialPage(0x100)).unwrap();
    assert_eq!(shown(&test), at("contents"), "it arrives where it names");
    test.core.show_address().unwrap();
    let sent = test.sent();
    assert!(sent.contains(&ToInterface::ShowContents(channel)), "{sent:?}");
    open(&mut test, channel, 0x200);
    assert_eq!((shown(&test), entries(&test)), (at("200"), 2));
    test.sent();
    test.core.intent(ToCore::Back).unwrap();
    assert_eq!(shown(&test), at("contents"));
    assert!(test.sent().contains(&ToInterface::ShowContents(channel)));
}

// REQ-1693 items 3 and 8: a location with no fragment inherits the tab's, and
// one naming a page's file has its own place.
#[test]
fn a_redirect_inherits_a_fragment_only_when_it_names_no_place() {
    let mut test = Test::new("address redirect fragment");
    test.core.session.open(&at("302?a=1")).unwrap();
    let channel = test.core.active_channel().unwrap();
    assert_eq!(test.core.session.server_redirect(channel, "https://other.invalid/a/").unwrap().text, "https://other.invalid/a/302?a=1");
    assert_eq!(test.core.session.server_redirect(channel, "https://other.invalid/a/400.tti").unwrap().text, "https://other.invalid/a/400");
    assert_eq!(test.core.session.server_redirect(channel, "https://other.invalid/a/500").unwrap().text, "https://other.invalid/a/500");
    assert!(test.core.session.server_redirect(channel, &format!("https://other.invalid/a/{}", neoteletext_web::channel::SETTINGS_FILE)).is_err());
}

// REQ-1693 item 6: page parameters reach the channel with the selection they
// came with, stay in the address for Reload, and go with another selection.
#[cfg(feature = "interaction")]
#[test]
fn page_parameters_go_with_their_selection() {
    let (mut test, channel) = arrived("parameters");
    let parameters = |test: &Test, channel| test.core.typing.parameters(channel).map(str::to_owned);
    test.core.session.navigate(&format!("{CHANNEL}100"), true).unwrap();
    assert_ne!(test.core.active_channel(), Some(channel));
    test.core.session.navigate(&at("200?a=1"), true).unwrap();
    let channel = test.core.active_channel().unwrap();
    test.core.show_address().unwrap();
    assert_eq!(test.core.typing.selection(channel), Some((0x200, SUBCODE_ANY)));
    assert_eq!(parameters(&test, channel).as_deref(), Some("a=1"));
    open(&mut test, channel, 0x200);
    assert_eq!(shown(&test), at("200?a=1"), "opening the page it names changes nothing");

    open(&mut test, channel, 0x300);
    assert_eq!(shown(&test), at("300"));
    assert_eq!(parameters(&test, channel).as_deref(), Some(""), "another page leaves them behind");

    test.core.intent(ToCore::Back).unwrap();
    assert_eq!(shown(&test), at("200?a=1"));
    assert_eq!(parameters(&test, channel).as_deref(), Some("a=1"), "an address sends them again");

    assert!(test.core.session.reload());
    let again = test.core.active_channel().unwrap();
    assert_ne!(again, channel);
    test.core.show_address().unwrap();
    assert_eq!(parameters(&test, again).as_deref(), Some("a=1"), "and Reload");
}

// REQ-1693 item 6, by a sub-page: a step leaves them behind in the address
// and the SELECT alike.
#[cfg(feature = "interaction")]
#[test]
fn a_sub_page_chosen_leaves_page_parameters_behind() {
    let mut test = Test::new("address parameters sub-page");
    test.core.session.open(&at("200/0002?a=1")).unwrap();
    let channel = test.core.active_channel().unwrap();
    test.core.show_address().unwrap();
    assert_eq!(test.core.typing.selection(channel), Some((0x200, 2)));
    assert_eq!(test.core.typing.parameters(channel), Some("a=1"));
    assert!(test.core.session.subpage_chosen(0x200, Some(3)));
    assert_eq!(shown(&test), at("200/0003"));
    test.core.show_address().unwrap();
    assert_eq!(test.core.typing.selection(channel), Some((0x200, 3)));
    assert_eq!(test.core.typing.parameters(channel), Some(""));
}

// REQ-1693 item 10: the start page is in no channel.
#[test]
fn the_start_page_keeps_its_address() {
    let mut test = Test::new("address start page");
    test.core.session.open("neoteletext:home").unwrap();
    let channel = test.core.active_channel().unwrap();
    open(&mut test, channel, 0x101);
    assert_eq!(shown(&test), "neoteletext:home");
}

// REQ-1690 item 7: during a redirect's notice the address stays.
#[test]
fn the_address_waits_for_a_redirect_to_arrive() {
    let (mut test, channel) = arrived("notice");
    let before = shown(&test);
    test.core.fetched(channel, FromFetcher::ProgramRedirect("https://new.invalid/".into())).unwrap();
    assert_eq!(shown(&test), before);
    assert_eq!(test.core.strip().redirecting, "https://new.invalid/");
}
