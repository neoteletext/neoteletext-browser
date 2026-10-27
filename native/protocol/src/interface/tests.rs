//! The core and interface pipe's tests: every message round trips, and
//! what either side may not send, or sends malformed, is refused.

use interface_core::browser::{CertificateProblem, StripState, TabInfo};
use interface_core::canvas::PageKey;
use interface_core::channel::{Channel, PageInfo, Status};

use super::kinds::{
    KIND_CELL, KIND_FRAME, KIND_FROM, KIND_GO_TO, KIND_HISTORY, KIND_HISTORY_VIEW, KIND_MUTE_ALL, KIND_NAVIGATE, KIND_NEW_WINDOW, KIND_PAGE, KIND_STRIP,
    KIND_TAB_TO_WINDOW, KIND_TABS, KIND_TARGET, KIND_VISIBLE,
};
use super::*;
use crate::{MAX_IDENTITY, MAX_PAYLOAD, NEW_WINDOW, ProtocolError, clean};
#[cfg(not(feature = "links"))]
use crate::{decoder, fetcher};

const KEY: PageKey = PageKey { channel: 7, page: 0x123 };

fn strip() -> StripState {
    StripState {
        address: "https://example.invalid/".into(),
        identity: "example.invalid".into(),
        encrypted: true,
        back: true,
        forward: false,
        bookmarked: true,
        local: false,
        built_in: false,
        private: true,
        notice: "Proxy changed to socks5://127.0.0.1:1080".into(),
        refused: Some(CertificateProblem::Revoked),
    }
}

/// Decision 83: a refused certificate's kind is one of seven codes;
/// any other is a broken message.
#[test]
fn a_certificate_problem_is_held_to_its_codes() {
    for problem in CertificateProblem::ALL {
        assert_eq!(problem_of(problem_code(problem)), Some(problem));
    }
    assert_eq!(problem_of(0), None);
    assert_eq!(problem_of(8), None);
    let mut bytes = Vec::new();
    write_to_interface(&mut bytes, &ToInterface::Strip(strip())).unwrap();
    let last = bytes.len() - 1;
    bytes[last] = 9;
    assert!(read_from_core(&mut bytes.as_slice()).is_err());
}

fn channel() -> Channel {
    let mut channel = Channel::empty();
    channel.channel = 9;
    channel.title = "Example".into();
    channel.banks[0] = "News".into();
    channel.pages[0] = Some(PageInfo { title: "Front page".into(), status: Status::Loaded });
    channel.pages[799] = Some(PageInfo { title: "Clock".into(), status: Status::Available });
    channel
}

/// Decision 72: window 0 is a new window's, never one's own; a tab's
/// places are held to the strip's bound.
#[test]
fn window_messages_are_held_to_their_bounds() {
    assert!(write_to_core(&mut Vec::new(), &ToCore::From(NEW_WINDOW)).is_err());
    assert!(write_to_interface(&mut Vec::new(), &ToInterface::OpenWindow { window: NEW_WINDOW, geometry: None }).is_err());
    assert!(write_to_core(&mut Vec::new(), &ToCore::TabToWindow { tab: 64, window: 1, index: 0 }).is_err());
    let refused = |bytes: &[u8]| read_from_interface(&mut &bytes[..]).is_err();
    assert!(refused(&[KIND_FROM, 0, 0, 0, 4, 0, 0, 0, 0]), "From window 0");
    assert!(refused(&[KIND_NEW_WINDOW, 0, 0, 0, 1, 2]), "a flag of 2");
    assert!(refused(&[KIND_TAB_TO_WINDOW, 0, 0, 0, 6, 0, 0, 0, 0, 1, 65]), "a place past the strip");
    let refused = |bytes: &[u8]| read_from_core(&mut &bytes[..]).is_err();
    assert!(refused(&[KIND_TARGET, 0, 0, 0, 4, 0, 0, 0, 0]), "Target window 0");
}

#[test]
fn every_message_to_the_interface_round_trips() {
    for message in [
        ToInterface::Sounding(vec![3, 7]),
        ToInterface::Tabs {
            tabs: vec![
                TabInfo { title: "news".into(), colour: 1, channel: 3, muted: false, private: false, address: "https://news.invalid/channel".into() },
                TabInfo { title: "sport".into(), colour: 7, channel: 9, muted: true, private: true, address: String::new() },
            ],
            active: 1,
        },
        ToInterface::Strip(strip()),
        ToInterface::Channel(channel()),
        ToInterface::Page { channel: 9, page: 0x899, title: "Clock".into() },
        ToInterface::SubPages { channel: 9, page: 0x210, subpages: vec![(1, "Story 1/2".into()), (2, "Comments".into())] },
        ToInterface::Updated { channel: 9, page: 0x302 },
        ToInterface::Frame { key: KEY, version: 3, image: vec![1, 2, 3] },
        ToInterface::Settings(settings::SettingsView {
            home: "neoteletext:home".into(),
            sound: false,
            volume: 30,
            proxy: settings::ProxyView::None,
            local: Vec::new(),
            restore_session: true,
            cookies: settings::CookieMode::Close,
            refusal: None,
        }),
        ToInterface::Update(settings::UpdateView { version: "0.1.0".into(), status: "NeoTeletext is up to date".into(), offer: None, checking: true }),
        ToInterface::Cookies(cookies::CookiesView { query: "news".into(), total: 0, ..cookies::CookiesView::default() }),
        ToInterface::PageControls(interface_core::controls::PageControls {
            page: Some(0x302),
            fastext: [true, false, true, false, true],
            reveal: true,
            hold: false,
            mix: true,
            subpages: 4,
            chosen: Some(2),
            subcode: 3,
        }),
        ToInterface::PageControls(interface_core::controls::PageControls::default()),
        ToInterface::Remote(settings::RemotePlace { mode: settings::RemoteMode::Floating, x: 800, y: 120 }),
        ToInterface::Target(2),
        ToInterface::OpenWindow { window: 3, geometry: None },
        ToInterface::OpenWindow { window: 4, geometry: Some(library::WindowGeometry { x: -40, y: 30, width: 900, height: 700, maximised: false }) },
        ToInterface::CloseWindow(3),
        ToInterface::History(library::HistoryView { query: "news".into(), total: 0, ..library::HistoryView::default() }),
        ToInterface::Suggestions(library::Suggestions { text: "n".into(), inline: "news.example".into(), rows: Vec::new() }),
        ToInterface::TabList(library::TabList { forward: true, entries: Vec::new(), chapters: Vec::new() }),
        ToInterface::Bookmarks(library::BookmarksView::default()),
        ToInterface::Star(library::StarView { open: true, ..library::StarView::default() }),
        ToInterface::Bar(library::BarView { shown: true, ..library::BarView::default() }),
        ToInterface::Console(console::ConsoleUpdate {
            channel: 2,
            replace: false,
            entries: vec![console::ConsoleEntry { tab: console::ConsoleTab::Log, level: console::Level::Warning, at_ms: 5, text: "x".into() }],
        }),
        ToInterface::Close,
    ] {
        let mut bytes = Vec::new();
        write_to_interface(&mut bytes, &message).unwrap();
        assert_eq!(read_from_core(&mut bytes.as_slice()).unwrap().unwrap(), message);
    }
}

#[test]
fn every_intent_round_trips() {
    for message in [
        ToCore::Visible(vec![KEY]),
        ToCore::Cell { key: KEY, version: 7, column: 39, row: 24 },
        ToCore::SelectTab(3),
        ToCore::CloseTab(0),
        ToCore::MoveTab { from: 3, to: 0 },
        ToCore::From(2),
        ToCore::NewWindow { private: false },
        ToCore::NewWindow { private: true },
        ToCore::TabToWindow { tab: 1, window: NEW_WINDOW, index: 0 },
        ToCore::TabToWindow { tab: 0, window: 7, index: 64 },
        ToCore::CloseWindow,
        ToCore::NewTab,
        ToCore::Navigate("sport.invalid".into()),
        ToCore::Back,
        ToCore::Forward,
        ToCore::Bookmark,
        ToCore::Home,
        ToCore::Reload,
        ToCore::PageControl(interface_core::controls::PageControl::Fastext(interface_core::controls::FastextKey::Index)),
        ToCore::PageControl(interface_core::controls::PageControl::SubPage(false)),
        ToCore::PageControl(interface_core::controls::PageControl::Latest),
        ToCore::PageControl(interface_core::controls::PageControl::SubPageOf { page: 0x210, subcode: 3 }),
        ToCore::Drawn,
        ToCore::InstallUpdate,
        ToCore::ChangeSetting(settings::SettingChange::Volume(40)),
        ToCore::Console(true),
        ToCore::History(library::HistoryRequest::Clear(library::ClearRange::All)),
        ToCore::Bookmarks(library::BookmarkRequest::Undo),
        ToCore::Star(library::StarRequest::Done),
        ToCore::Bar(library::BarRequest::Show(true)),
        ToCore::Suggest("news".into()),
        ToCore::Suggestion { number: 4, new_tab: true },
        ToCore::TabList(false),
        ToCore::GoTo { forward: true, steps: 15 },
        ToCore::NewPrivateTab,
        ToCore::Window(library::WindowGeometry { x: 10, y: 20, width: 1440, height: 900, maximised: false }),
        ToCore::Cookies(cookies::CookiesRequest::Block { number: 3, blocked: true }),
    ] {
        let mut bytes = Vec::new();
        write_to_core(&mut bytes, &message).unwrap();
        assert_eq!(read_from_interface(&mut bytes.as_slice()).unwrap().unwrap(), message);
    }
}

#[test]
fn neither_side_accepts_the_other_sides_kinds() {
    // An interface sending the trusted strip's state to the trusted core.
    let mut bytes = Vec::new();
    write_to_interface(&mut bytes, &ToInterface::Strip(strip())).unwrap();
    assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(KIND_STRIP))));
    bytes.clear();
    write_to_core(&mut bytes, &ToCore::Navigate("x".into())).unwrap();
    assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(KIND_NAVIGATE))));
    // The interface sending history as if it were the trusted core's.
    bytes.clear();
    write_to_interface(&mut bytes, &ToInterface::History(library::HistoryView::default())).unwrap();
    assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(KIND_HISTORY_VIEW))));
    bytes.clear();
    write_to_core(&mut bytes, &ToCore::History(library::HistoryRequest::Panel(true))).unwrap();
    assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(KIND_HISTORY))));
    // Steps a list never offers.
    assert!(write_to_core(&mut Vec::new(), &ToCore::GoTo { forward: false, steps: 0 }).is_err());
    let beyond = u8::try_from(library::MAX_STEPS + 1).unwrap();
    assert!(read_from_interface(&mut [KIND_GO_TO, 0, 0, 0, 2, 0, beyond].as_slice()).is_err());
}

#[test]
fn oversized_and_malformed_payloads_are_refused() {
    let mut bytes = vec![KIND_FRAME];
    bytes.extend_from_slice(&u32::try_from(MAX_PAYLOAD + 1).unwrap().to_be_bytes());
    assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::TooLong(_))));
    // A cell outside the 40 by 25 page.
    let mut bytes = vec![KIND_CELL, 0, 0, 0, 12, 0, 0, 0, 7, 0x01, 0x23, 0, 0, 0, 1, 40, 0];
    assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::BadValue("cell outside the page"))));
    // A page count larger than the payload holds.
    bytes = vec![KIND_VISIBLE, 0, 0, 0, 2, 0, 1];
    assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::BadValue("payload ends early"))));
    // Trailing bytes.
    bytes = vec![KIND_VISIBLE, 0, 0, 0, 3, 0, 0, 9];
    assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::BadValue("payload longer than its fields"))));
    // A page number outside 100 to 8FF.
    bytes = vec![KIND_VISIBLE, 0, 0, 0, 8, 0, 1, 0, 0, 0, 1, 0x09, 0x00];
    assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::BadValue("page number"))));
    // A new page with a hexadecimal number, and unknown strip flags.
    bytes = vec![KIND_PAGE, 0, 0, 0, 8, 0, 0, 0, 9, 0x1A, 0x00, 0, 0];
    assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::BadValue("page number"))));
    bytes = vec![KIND_STRIP, 0, 0, 0, 7, 0x80, 0, 0, 0, 0, 0, 0];
    assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::BadValue("strip flags"))));
    // A tab colour the interface has no colour for.
    bytes = vec![KIND_TABS, 0, 0, 0, 10, 0, 1, 8, 0, 0, 0, 1, 0, 0, 0];
    assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::BadValue("tab colour"))));
    // Unknown tab flags, and a mute that is neither on nor off.
    bytes = vec![KIND_TABS, 0, 0, 0, 10, 0, 1, 0, 0, 0, 0, 1, 4, 0, 0];
    assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::BadValue("tab flags"))));
    bytes = vec![KIND_MUTE_ALL, 0, 0, 0, 1, 2];
    assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::BadValue("mute"))));
}

/// With hyperlinks compiled out, no side accepts their kinds.
#[cfg(not(feature = "links"))]
#[test]
fn with_links_compiled_out_their_kinds_are_refused() {
    for kind in 11..=15 {
        let bytes = [kind, 0, 0, 0, 0];
        assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(refused)) if refused == kind));
    }
    #[cfg(not(feature = "interaction"))]
    for kind in [26, 27] {
        let bytes = [kind, 0, 0, 0, 0];
        assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(refused)) if refused == kind));
    }
    for kind in [24, 25, 36, 37] {
        let bytes = [kind, 0, 0, 0, 0];
        assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(refused)) if refused == kind));
    }
    let bytes = [68, 0, 0, 0, 0];
    assert!(matches!(decoder::read_from_decoder(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(68))));
    let bytes = [83, 0, 0, 0, 0];
    assert!(matches!(fetcher::read_from_fetcher(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(83))));
}

#[test]
fn a_page_listed_twice_is_refused() {
    let mut bytes = Vec::new();
    write_to_interface(&mut bytes, &ToInterface::Channel(channel())).unwrap();
    // Make the second entry's page number (899) equal the first's (100).
    let second = bytes.windows(2).rposition(|pair| pair == [0x08, 0x99]).unwrap();
    bytes[second] = 0x01;
    bytes[second + 1] = 0x00;
    assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::BadValue("page listed twice"))));
}

#[test]
fn text_is_cleaned_before_sending_and_checked_on_arrival() {
    let hostile = "exa\u{202E}moc.\u{200B}elpmaxe\u{0007}";
    assert_eq!(clean(hostile, MAX_IDENTITY), "examoc.elpmaxe");
    assert_eq!(clean("ééé", 5), "éé", "cut on a character boundary");
    let mut bytes = Vec::new();
    let strip = ToInterface::Strip(StripState { identity: hostile.into(), ..StripState::default() });
    assert!(matches!(write_to_interface(&mut bytes, &strip), Err(ProtocolError::BadValue("text not cleaned"))));
    // Written by hand, as a faulty sender would, it is still refused.
    let identity = hostile.as_bytes();
    let mut payload = vec![0];
    payload.extend_from_slice(&u16::try_from(identity.len()).unwrap().to_be_bytes());
    payload.extend_from_slice(identity);
    payload.extend_from_slice(&[0, 0]);
    let mut bytes = vec![KIND_STRIP];
    bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(&payload);
    assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::BadValue("text not cleaned"))));
}

/// Finding F-29: a page key's number is a display page, 100 to 899 with
/// decimal digits, as the decoder takes it; a hexadecimal one such as
/// 1A0 is refused where it arrives, not by the decoder after the trusted
/// core sent it on.
#[test]
fn a_page_key_with_a_hexadecimal_number_is_refused() {
    let opened = |page: u16| {
        let mut bytes = Vec::new();
        write_to_core(&mut bytes, &ToCore::Opened(Some(PageKey { channel: 7, page }))).unwrap();
        read_from_interface(&mut bytes.as_slice())
    };
    assert_eq!(opened(0x899).unwrap(), Some(ToCore::Opened(Some(PageKey { channel: 7, page: 0x899 }))));
    for page in [0x1A0, 0x10F, 0x8FF, 0x0FF, 0x900] {
        assert!(opened(page).is_err(), "{page:X}");
    }
}
