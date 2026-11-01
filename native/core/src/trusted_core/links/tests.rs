//! REQ-1670: every kind of link to another channel moves the tab at once,
//! through the intents the interface sends, against the stand-in broker
//! the new channel's helpers need.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]

use confine::broker::stand_in::StandIn;
use interface_core::canvas::PageKey;
use interface_core::controls::{FastextKey, PageControl};
use neoteletext::links::{Channel, DWELL_MILLISECONDS};
use protocol::ToCore;
use protocol::links::{Alias, Candidate, Fastext, FastextTarget, NumberLink, Service, Spot};

use super::super::Core;
use super::super::test_support::core_on;

const HOME: &str = "https://news.invalid/teletext/100";
const ELSEWHERE: &str = "https://weather.invalid/teletext/";
const PAGE: u16 = 0x100;
/// The row and first column of the page's one link.
const LINK_ROW: u8 = 5;
const LINK_COLUMN: u8 = 0;
const LINK_LENGTH: u8 = 7;
const VERSION: u32 = 1;

fn link(target: &str) -> Candidate {
    Candidate { row: LINK_ROW, column: LINK_COLUMN, length: LINK_LENGTH, double: false, light: false, target: target.into() }
}

struct Viewer {
    core: Core,
    /// Kept for the broker's far end, which ends with it.
    _stand_in: StandIn,
    key: PageKey,
}

impl Viewer {
    /// A tab on `address`, page 100 open, older than a link's half second.
    fn on(address: &str) -> Self {
        let (mut core, stand_in) = core_on(address);
        let key = PageKey { channel: core.session.tabs[0].channel.id, page: PAGE };
        core.intent(ToCore::Opened(Some(key))).unwrap();
        Self { core, _stand_in: stand_in, key }
    }

    /// The page shows `candidates`, `numbers` and `fastext`, in an image
    /// sent long enough ago for its links to have settled.
    fn page(&mut self, candidates: Vec<Candidate>, numbers: Vec<NumberLink>, fastext: Fastext, version: u32, sent: u64) {
        let address = self.core.session.tabs[0].address().unwrap().clone();
        let channel = Channel { id: self.key.channel, address: &address };
        self.core.links.found(self.key, candidates, numbers, Vec::new(), fastext);
        self.core.links.image(self.key, version, &channel, sent);
    }

    fn click(&mut self, version: u32) {
        let spot = Spot { key: self.key, version, column: LINK_COLUMN + 1, row: LINK_ROW };
        self.core.intent(ToCore::Point(Some(spot))).unwrap();
        self.core.intent(ToCore::Cell { key: self.key, version, column: spot.column, row: spot.row }).unwrap();
    }

    fn address(&self) -> &str {
        &self.core.session.active_tab().unwrap().address().unwrap().text
    }

    /// The tab went to `to` at once, as a page-started navigation
    /// (REQ-725), and nothing asked; Back returns to `from`.
    fn went(&mut self, to: &str, from: &str) {
        assert_eq!(self.address(), to, "the tab moved at once");
        assert_eq!(self.core.links.prompt(), None, "no question");
        assert_eq!(self.core.strip().redirecting, "", "REQ-1686: a followed link shows no notice");
        assert!(self.core.session.active_tab().unwrap().pending.as_ref().is_some_and(|pending| !pending.typed), "recorded as the page's");
        self.core.intent(ToCore::Back).unwrap();
        assert_eq!(self.address(), from, "Back returns");
    }
}

// REQ-1670: a text link.
#[test]
fn a_text_link_to_another_channel_goes_at_once() {
    let mut viewer = Viewer::on(HOME);
    viewer.page(vec![link(ELSEWHERE)], Vec::new(), Fastext::default(), VERSION, 0);
    viewer.click(VERSION);
    viewer.went(ELSEWHERE, HOME);
}

// REQ-1670: a number keyed that the page's LK line leads elsewhere.
#[test]
fn a_keyed_number_link_to_another_channel_goes_at_once() {
    let mut viewer = Viewer::on(HOME);
    viewer.page(Vec::new(), vec![NumberLink { number: 0x302, target: ELSEWHERE.into() }], Fastext::default(), VERSION, 0);
    viewer.core.intent(ToCore::Typed { number: 0x302, opened: Some((viewer.key, VERSION)) }).unwrap();
    viewer.went(ELSEWHERE, HOME);
}

// REQ-1670: a coloured key and Index, whose target is a listed service.
#[test]
fn a_coloured_key_and_index_to_another_channel_go_at_once() {
    for key in [FastextKey::Green, FastextKey::Index] {
        let mut viewer = Viewer::on(HOME);
        let channel = viewer.key.channel;
        viewer.core.links.descriptor(channel, &[], &[Service { name: "weather".into(), address: ELSEWHERE.into() }]);
        let target = Some(FastextTarget { page: 0x100, subcode: None, service: Some(0) });
        viewer.page(Vec::new(), Vec::new(), Fastext { keys: [target, target, target, target, target], row24: true, prompts: [None; 4] }, VERSION, 0);
        viewer.core.intent(ToCore::PageControl(PageControl::Fastext(key))).unwrap();
        viewer.went("https://weather.invalid/teletext/100", HOME);
    }
}

// REQ-1670: a number a descriptor alias leads with.
#[test]
fn a_descriptor_alias_to_another_channel_goes_at_once() {
    let mut viewer = Viewer::on(HOME);
    let channel = viewer.key.channel;
    viewer.core.links.descriptor(channel, &[Alias { page: 0x500, target: ELSEWHERE.into() }], &[]);
    viewer.core.intent(ToCore::Typed { number: 0x500, opened: None }).unwrap();
    viewer.went(ELSEWHERE, HOME);
}

// REQ-1670: a named service, `name:mpp`.
#[test]
fn a_named_service_goes_at_once() {
    let mut viewer = Viewer::on(HOME);
    let channel = viewer.key.channel;
    viewer.core.links.descriptor(channel, &[], &[Service { name: "sport".into(), address: "https://sport.invalid/".into() }]);
    viewer.page(vec![link("sport:302")], Vec::new(), Fastext::default(), VERSION, 0);
    viewer.click(VERSION);
    viewer.went("https://sport.invalid/302", HOME);
}

// REQ-1670: a bookmark chosen on the built-in start page.
#[test]
fn a_start_page_bookmark_goes_at_once() {
    let mut viewer = Viewer::on(neoteletext::address::START_PAGE);
    viewer.core.session.bookmarks.add(neoteletext::bookmarks::BAR, "News", HOME).unwrap();
    let key = PageKey { channel: viewer.key.channel, page: 0x201 };
    viewer.core.intent(ToCore::Opened(Some(key))).unwrap();
    assert_eq!(viewer.address(), HOME, "the tab moved at once");
    assert_eq!(viewer.core.links.prompt(), None, "no question");
    viewer.core.intent(ToCore::Back).unwrap();
    assert!(viewer.core.session.active_tab().unwrap().address().is_some_and(neoteletext::address::Address::is_start_page), "Back returns");
}

// REQ-1670: a local file's link to a channel.
#[test]
fn a_files_link_to_a_channel_goes_at_once() {
    let file = if cfg!(windows) { r"C:\teletext\recording.t42" } else { "/teletext/recording.t42" };
    let mut viewer = Viewer::on(file);
    let from = viewer.address().to_owned();
    viewer.page(vec![link(ELSEWHERE)], Vec::new(), Fastext::default(), VERSION, 0);
    viewer.click(VERSION);
    viewer.went(ELSEWHERE, &from);
}

// REQ-1670, REQ-807: a link that changed under the pointer is not followed.
#[test]
fn a_link_changed_under_the_pointer_is_not_followed() {
    let mut viewer = Viewer::on(HOME);
    viewer.page(vec![link(ELSEWHERE)], Vec::new(), Fastext::default(), VERSION, 0);
    let spot = Spot { key: viewer.key, version: VERSION, column: LINK_COLUMN + 1, row: LINK_ROW };
    viewer.core.intent(ToCore::Point(Some(spot))).unwrap();
    // The page swaps the link's target this moment, under the pointer.
    let now = viewer.core.now();
    viewer.page(vec![link("https://other.invalid/")], Vec::new(), Fastext::default(), VERSION + 1, now);
    viewer.core.intent(ToCore::Point(Some(Spot { version: VERSION + 1, ..spot }))).unwrap();
    viewer.core.intent(ToCore::Cell { key: viewer.key, version: VERSION + 1, column: spot.column, row: spot.row }).unwrap();
    assert_eq!(viewer.address(), HOME, "not followed within its half second of changing ({DWELL_MILLISECONDS} ms)");
}

// REQ-1660 item 3, REQ-1690: a link naming a sub-page opens it, and the
// address says so: on the page open, or on another.
#[test]
fn a_link_to_a_sub_page_opens_it() {
    let mut viewer = Viewer::on(HOME);
    viewer.page(vec![link("100:0002")], Vec::new(), Fastext::default(), VERSION, 0);
    viewer.click(VERSION);
    assert_eq!(viewer.address(), "https://news.invalid/teletext/100/0002");
    #[cfg(feature = "interaction")]
    assert_eq!(viewer.core.typing.selection(viewer.key.channel), Some((PAGE, 2)));
    viewer.page(vec![link("302:0002")], Vec::new(), Fastext::default(), VERSION + 1, 0);
    viewer.click(VERSION + 1);
    assert_eq!(viewer.address(), "https://news.invalid/teletext/302/0002");
    viewer.core.intent(ToCore::Opened(Some(PageKey { page: 0x302, ..viewer.key }))).unwrap();
    #[cfg(feature = "interaction")]
    assert_eq!(viewer.core.typing.selection(viewer.key.channel), Some((0x302, 2)), "when it opens");
}

// REQ-808, REQ-1670: a `web:` link keeps its prompt, and the tab stays.
#[test]
fn a_web_link_still_asks() {
    let mut viewer = Viewer::on(HOME);
    viewer.page(vec![link("web:https://www.example.invalid/")], Vec::new(), Fastext::default(), VERSION, 0);
    viewer.click(VERSION);
    assert_eq!(viewer.address(), HOME);
    assert_eq!(viewer.core.links.prompt().map(|prompt| prompt.destination), Some("https://www.example.invalid/".to_owned()));
}

// REQ-685: a cell with no link in the core's table follows nothing.
#[test]
fn a_click_where_the_core_holds_no_link_moves_nothing() {
    let mut viewer = Viewer::on(HOME);
    viewer.page(Vec::new(), Vec::new(), Fastext::default(), VERSION, 0);
    viewer.click(VERSION);
    assert_eq!(viewer.address(), HOME);
}
