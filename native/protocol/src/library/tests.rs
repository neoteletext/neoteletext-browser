//! The library's tests: every view and request round trips, the bounds
//! hold both ways, and the largest views fit one message.
#![allow(clippy::unwrap_used, reason = "a test")]

use super::bookmarks::BAR_SHOW;
use super::history::{HISTORY_CLEAR, HISTORY_REMOVE};
use super::*;
use crate::MAX_PAYLOAD;

fn item(number: u32) -> ItemRow {
    ItemRow {
        number,
        folder: number.is_multiple_of(2),
        name: "News Example".into(),
        address: "https://news.example/teletext/".into(),
        place: "Bookmarks bar".into(),
    }
}

fn folder(number: u32, depth: u8) -> FolderRow {
    FolderRow { number, name: "Weather".into(), depth }
}

fn history() -> HistoryView {
    HistoryView {
        query: "news".into(),
        first: 0,
        total: 1,
        rows: vec![HistoryRow {
            number: 7,
            heading: Some("Today: Wednesday 22 October 2026".into()),
            time: "14:05".into(),
            channel: "News Example".into(),
            page: Some(0x302),
            title: "WEATHER".into(),
            address: "https://news.example/teletext/#302".into(),
            bookmarked: true,
        }],
        closed: vec![ClosedTab { number: 3, title: "Sport".into() }],
        notice: String::new(),
    }
}

fn round_trip<T: PartialEq + std::fmt::Debug>(
    value: &T,
    push: fn(&mut Vec<u8>, &T) -> Result<(), ProtocolError>,
    read: fn(&mut Fields<'_>) -> Result<T, ProtocolError>,
) {
    let mut payload = Vec::new();
    push(&mut payload, value).unwrap();
    let mut fields = Fields::new(&payload);
    assert_eq!(&read(&mut fields).unwrap(), value);
    fields.end().unwrap();
}

/// Every view and request survives the pipe.
#[test]
fn every_view_and_request_round_trips() {
    round_trip(&history(), push_history_view, read_history_view);
    round_trip(
        &Suggestions {
            text: "news".into(),
            inline: "news.example/teletext/".into(),
            rows: vec![Suggestion { number: 1, address: "https://news.example/".into(), title: "News".into(), bookmarked: true }],
        },
        push_suggestions,
        read_suggestions,
    );
    round_trip(
        &TabList {
            forward: false,
            entries: vec![TabEntry { steps: 2, page: None, title: "Sport".into(), address: "https://sport.example/".into() }],
            chapters: vec![TabEntry { steps: 30, page: Some(0x100), title: "News".into(), address: "https://news.example/".into() }],
        },
        push_tab_list,
        read_tab_list,
    );
    round_trip(
        &BookmarksView {
            path: vec![folder(1, 0), folder(4, 1)],
            folders: vec![folder(1, 0), folder(2, 0), folder(4, 1)],
            folders_first: 0,
            folders_total: 3,
            query: String::new(),
            items: vec![item(5), item(6)],
            items_first: 0,
            items_total: 2,
            undo: "Removed News".into(),
            notice: "Exported".into(),
        },
        push_bookmarks_view,
        read_bookmarks_view,
    );
    round_trip(&StarView { open: true, name: "News".into(), folder: 2, folders: vec![folder(1, 0), folder(2, 0)], page: true }, push_star_view, read_star_view);
    round_trip(&BarView { shown: true, items: vec![item(1)], open: Some((2, vec![item(3)])) }, push_bar_view, read_bar_view);
    for request in [
        HistoryRequest::Panel(true),
        HistoryRequest::Query { text: "x".into(), first: 100 },
        HistoryRequest::Open { number: 4, new_tab: true },
        HistoryRequest::Remove(vec![1, 2, 3]),
        HistoryRequest::MoreFromSite(9),
        HistoryRequest::RemoveBookmark(9),
        HistoryRequest::Clear(ClearRange::FourWeeks),
        HistoryRequest::ClearCookies(ClearRange::Hour),
        HistoryRequest::Clear(ClearRange::FifteenMinutes),
        HistoryRequest::Reopen(None),
        HistoryRequest::Reopen(Some(3)),
    ] {
        round_trip(&request, push_history_request, read_history_request);
    }
    for request in [
        BookmarkRequest::Panel(false),
        BookmarkRequest::Folder(4),
        BookmarkRequest::Scroll { folders_first: 100, items_first: 200 },
        BookmarkRequest::Query("news".into()),
        BookmarkRequest::Open { number: 5, new_tab: false },
        BookmarkRequest::Edit { number: 5, name: "News".into(), address: "news.example".into() },
        BookmarkRequest::Move { number: 5, folder: 1, index: 0 },
        BookmarkRequest::NewFolder("Weather".into()),
        BookmarkRequest::Add { name: "Sport".into(), address: "sport.example".into() },
        BookmarkRequest::Remove(5),
        BookmarkRequest::Undo,
        BookmarkRequest::Sort,
        BookmarkRequest::Import("/home/user/bookmarks.txt".into()),
        BookmarkRequest::Export,
    ] {
        round_trip(&request, push_bookmark_request, read_bookmark_request);
    }
    for request in [StarRequest::Edit { name: "News".into(), folder: 2 }, StarRequest::WholeChannel, StarRequest::Remove, StarRequest::Done] {
        round_trip(&request, push_star_request, read_star_request);
    }
    round_trip(&WindowGeometry { x: -120, y: 40, width: 1440, height: 900, maximised: true }, push_window, read_window);
    assert!(push_window(&mut Vec::new(), &WindowGeometry { x: 0, y: 0, width: 0, height: 900, maximised: false }).is_err());
    assert!(push_window(&mut Vec::new(), &WindowGeometry { x: 40_000, y: 0, width: 10, height: 900, maximised: false }).is_err());
    for request in [BarRequest::Show(true), BarRequest::Folder(None), BarRequest::Folder(Some(4)), BarRequest::Open { number: 1, new_tab: true }] {
        let mut payload = Vec::new();
        push_bar_request(&mut payload, &request);
        assert_eq!(read_bar_request(&mut Fields::new(&payload)).unwrap(), request);
    }
}

/// The bounds hold both ways: too many rows, too deep a folder, a page
/// that is no page, text that was not cleaned, an unknown request.
#[test]
fn what_breaks_a_bound_is_refused() {
    let mut view = history();
    view.rows = vec![view.rows[0].clone(); MAX_VIEW_ROWS + 1];
    assert!(push_history_view(&mut Vec::new(), &view).is_err());
    let mut view = history();
    view.rows[0].title = "a\u{202E}b".into();
    assert!(push_history_view(&mut Vec::new(), &view).is_err(), "hidden characters");
    let mut view = history();
    view.rows[0].page = Some(0x0FF);
    assert!(push_history_view(&mut Vec::new(), &view).is_err());
    assert!(push_folder(&mut Vec::new(), &folder(1, u8::try_from(MAX_FOLDER_DEPTH + 1).unwrap())).is_err());
    assert!(read_history_request(&mut Fields::new(&[HISTORY_CLEAR, 7])).is_err(), "no seventh range");
    assert!(read_history_request(&mut Fields::new(&[99])).is_err());
    assert!(read_bookmark_request(&mut Fields::new(&[99])).is_err());
    assert!(read_star_request(&mut Fields::new(&[99])).is_err());
    assert!(read_bar_request(&mut Fields::new(&[BAR_SHOW, 2])).is_err());
    let too_many = u16::try_from(MAX_VIEW_ROWS + 1).unwrap().to_be_bytes();
    assert!(read_history_request(&mut Fields::new(&[HISTORY_REMOVE, too_many[0], too_many[1]])).is_err());
    assert!(read_tab_list(&mut Fields::new(&[0, 0, 1, 0, 0, 0])).is_err(), "an entry no steps away");
}

/// Section 2.4: a whole view of the longest rows fits one message.
#[test]
fn the_largest_views_fit_one_message() {
    let long = |bytes: usize| "x".repeat(bytes);
    let row = HistoryRow {
        number: u32::MAX,
        heading: Some(long(MAX_LABEL)),
        time: long(MAX_LABEL),
        channel: long(MAX_NAME),
        page: Some(0x899),
        title: long(MAX_NAME),
        address: long(MAX_SHOWN_ADDRESS),
        bookmarked: true,
    };
    let view = HistoryView {
        query: long(MAX_QUERY),
        first: 0,
        total: u32::MAX,
        rows: vec![row; MAX_VIEW_ROWS],
        closed: vec![ClosedTab { number: 1, title: long(MAX_NAME) }; MAX_CLOSED],
        notice: long(MAX_NOTICE),
    };
    let mut payload = Vec::new();
    push_history_view(&mut payload, &view).unwrap();
    assert!(payload.len() <= MAX_PAYLOAD, "{}", payload.len());
    let item = ItemRow { number: 1, folder: false, name: long(MAX_NAME), address: long(MAX_SHOWN_ADDRESS), place: long(MAX_LABEL) };
    let folders = vec![FolderRow { number: 1, name: long(MAX_NAME), depth: 1 }; MAX_VIEW_ROWS];
    let view = BookmarksView {
        path: vec![FolderRow { number: 1, name: long(MAX_NAME), depth: 1 }; MAX_FOLDER_DEPTH + 1],
        folders,
        folders_first: 0,
        folders_total: 0,
        query: long(MAX_QUERY),
        items: vec![item.clone(); MAX_VIEW_ROWS],
        items_first: 0,
        items_total: 0,
        undo: long(MAX_NOTICE),
        notice: long(MAX_NOTICE),
    };
    let mut payload = Vec::new();
    push_bookmarks_view(&mut payload, &view).unwrap();
    assert!(payload.len() <= MAX_PAYLOAD, "{}", payload.len());
    let bar = BarView { shown: true, items: vec![item.clone(); MAX_VIEW_ROWS], open: Some((1, vec![item; MAX_VIEW_ROWS])) };
    let mut payload = Vec::new();
    push_bar_view(&mut payload, &bar).unwrap();
    assert!(payload.len() <= MAX_PAYLOAD, "{}", payload.len());
}
