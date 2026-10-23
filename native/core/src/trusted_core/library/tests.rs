//! Tests of history and bookmarks in the trusted core (requirements 20.1
//! to 20.20).
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]

use super::super::test_support::Test;
use super::*;
use neoteletext::bookmarks::{BAR, OTHER};
use protocol::ToInterface;
use protocol::decoder::BookmarkEntry;
use protocol::library::{BarRequest, BookmarkRequest, ClearRange, HistoryRequest, StarRequest};

/// Requirement 20.1: a visit is recorded when its page draws; not in a
/// private tab, not the browser's own page, not while history is not
/// remembered.
#[test]
fn a_visit_is_recorded_once_its_page_draws() {
    let mut test = Test::new("record");
    test.core.session.open("news.invalid").unwrap();
    assert!(test.core.library.history.is_empty(), "not before it draws");
    let channel = test.core.session.tabs[0].channel.id;
    test.core.record_pending(channel).unwrap();
    assert_eq!(test.core.library.history.len(), 1);
    test.core.record_pending(channel).unwrap();
    assert_eq!(test.core.library.history.query("")[0].visits, 1, "one draw is one visit");
    assert!(std::fs::read_to_string(test.folder.join("history.txt")).unwrap().contains("https://news.invalid/"), "appended");
    test.core.session.open_private("secret.invalid").unwrap();
    let private = test.core.session.tabs[1].channel.id;
    test.core.record_pending(private).unwrap();
    test.visit("neoteletext:home");
    assert_eq!(test.core.library.history.len(), 1, "{:?}", test.core.library.history.query(""));
    assert_eq!(test.core.session.tabs[0].history[0].title, "news.invalid", "the tab's entry has its title");
}

/// Requirements 20.4 to 20.6: rows by their view's numbers only; a
/// removal is in the file before the view says so.
#[test]
fn rows_are_removed_by_their_numbers_and_from_the_file() {
    let mut test = Test::new("remove");
    test.visit("news.invalid");
    test.visit("sport.invalid");
    test.core.history_request(HistoryRequest::Panel(true)).unwrap();
    let first = test.last_history();
    assert_eq!((first.total, first.rows.len()), (2, 2));
    assert!(first.rows[0].heading.as_deref().unwrap().starts_with("Today: "), "{:?}", first.rows[0].heading);
    assert_eq!(first.rows[1].heading, None, "one heading a day");
    test.core.history_request(HistoryRequest::Query { text: "SPORT".into(), first: 0 }).unwrap();
    let second = test.last_history();
    assert_eq!(second.rows.len(), 1);
    // A number from the earlier view names nothing now.
    test.core.history_request(HistoryRequest::Remove(vec![first.rows[0].number, first.rows[1].number])).unwrap();
    assert_eq!(test.core.library.history.len(), 2);
    // Each answer is a new view, with new numbers.
    test.core.history_request(HistoryRequest::Remove(vec![second.rows[0].number])).unwrap();
    assert_eq!(test.core.library.history.len(), 2, "the second view's number is stale too");
    let latest = test.last_history();
    assert_eq!(latest.rows[0].address, second.rows[0].address);
    test.core.history_request(HistoryRequest::Remove(vec![latest.rows[0].number])).unwrap();
    assert_eq!(test.core.library.history.len(), 1);
    let file = std::fs::read_to_string(test.folder.join("history.txt")).unwrap();
    assert!(!file.contains("sport.invalid"), "gone from the file: {file}");
    test.core.session.close(0);
    test.core.history_request(HistoryRequest::Clear(ClearRange::All)).unwrap();
    let cleared = test.last_history();
    assert!(cleared.rows.is_empty() && cleared.closed.is_empty(), "everything, and the closed tabs");
}

/// Decision 64: a bookmarked row says so, and Chrome's "Remove bookmark"
/// on it removes every bookmark of its address.
#[test]
fn a_rows_bookmarks_are_removed_from_its_menu() {
    let mut test = Test::new("unbookmark");
    test.visit("news.invalid");
    let address = test.core.library.history.query("")[0].address.clone();
    let other = test.core.session.bookmarks.folders()[0].0;
    test.core.session.bookmarks.add(other, "News", &address).unwrap();
    test.core.session.bookmarks.add(other, "News again", &address).unwrap();
    test.core.history_request(HistoryRequest::Panel(true)).unwrap();
    let view = test.last_history();
    assert!(view.rows[0].bookmarked);
    test.core.history_request(HistoryRequest::RemoveBookmark(view.rows[0].number)).unwrap();
    assert_eq!(test.core.session.bookmarks.find(&address), None, "both gone");
    assert!(!test.last_history().rows[0].bookmarked);
}

/// Requirement 20.11: bookmarks first, then history; inline only from typed.
#[test]
fn suggestions_come_from_bookmarks_then_history() {
    let mut test = Test::new("suggest");
    test.core.session.bookmarks.add(BAR, "News bookmark", "news.invalid/#302").unwrap();
    test.visit("news.invalid");
    test.sent();
    test.core.suggest("news").unwrap();
    let suggestions = test.sent().into_iter().find_map(|message| if let ToInterface::Suggestions(view) = message { Some(view) } else { None }).unwrap();
    let addresses: Vec<&str> = suggestions.rows.iter().map(|row| row.address.as_str()).collect();
    assert_eq!(addresses, ["https://news.invalid/#302", "https://news.invalid/"]);
    assert!(suggestions.rows[0].bookmarked && !suggestions.rows[1].bookmarked);
    assert_eq!(suggestions.inline, "", "a visit not typed never completes the field");
    // A test core has no broker to start decoders: the navigation is
    // made, and starting its decoder then fails.
    let _ = test.core.open_suggestion(suggestions.rows[1].number, false);
    assert_eq!(test.core.session.strip().address, "https://news.invalid/");
    let _ = test.core.open_suggestion(suggestions.rows[0].number, false);
    assert_eq!(test.core.session.strip().address, "https://news.invalid/", "a used view's number names nothing");
}

/// Requirement 20.16: the star bookmarks the page shown; the popover
/// keeps the whole channel or removes it.
#[test]
fn the_star_bookmarks_the_page_shown() {
    let mut test = Test::new("star");
    test.core.session.open("news.invalid/#302").unwrap();
    test.core.star().unwrap();
    let id = test.core.session.bookmarks.find("https://news.invalid/#302").unwrap();
    assert_eq!(test.core.session.bookmarks.item(id).unwrap().parent, OTHER);
    assert!(test.core.session.strip().bookmarked);
    let star = test.sent().into_iter().rev().find_map(|message| if let ToInterface::Star(view) = message { Some(view) } else { None }).unwrap();
    assert!(star.open && star.page);
    let bar = star.folders.iter().find(|folder| folder.name == "Bookmarks bar").unwrap().number;
    test.core.star_request(StarRequest::Edit { name: "Weather".into(), folder: bar }).unwrap();
    assert_eq!(test.core.session.bookmarks.item(id).unwrap().parent, BAR);
    test.core.star_request(StarRequest::WholeChannel).unwrap();
    assert_eq!(test.core.session.bookmarks.item(id).unwrap().address.as_deref(), Some("https://news.invalid/"));
    assert!(!test.core.session.strip().bookmarked, "the page is no longer the bookmark's");
    test.core.star_request(StarRequest::Remove).unwrap();
    assert!(test.core.session.bookmarks.is_empty());
    assert!(std::fs::read_to_string(test.folder.join("bookmarks.txt")).unwrap().lines().count() == 3, "the two folders and the version");
}

/// Decision 71: what a decoder read from another browser's file comes
/// in under Imported, its folders kept, its names cleaned, and only
/// what the address rules accept; what is left out is counted.
#[test]
fn another_browsers_bookmarks_come_in_as_channels_only() {
    let mut test = Test::new("netscape");
    let link = |name: &str, address: &str| BookmarkEntry::Link { name: name.into(), address: address.into() };
    let entries = vec![
        BookmarkEntry::Folder("Bookmarks bar".into()),
        link("Teefax\u{202e}", "https://teastop.co.uk/teefax/"),
        link("Most Visited", "place:sort=8"),
        link("A script", "javascript:alert(1)"),
        BookmarkEntry::FolderEnd,
        link("News", "news.invalid/#302"),
    ];
    test.core.bookmarks_read(Ok((entries, 2))).unwrap();
    let bookmarks = &test.core.session.bookmarks;
    let imported = bookmarks.children(OTHER).iter().copied().find(|&id| bookmarks.item(id).unwrap().name == "Imported").unwrap();
    let names = |folder: u32| bookmarks.children(folder).iter().map(|&id| bookmarks.item(id).unwrap().name.clone()).collect::<Vec<_>>();
    assert_eq!(names(imported), ["Bookmarks bar", "News"]);
    let bar = bookmarks.children(imported)[0];
    assert_eq!(names(bar), ["Teefax"], "the name cleaned; place: and javascript: refused");
    assert_eq!(test.core.library.manager_notice, notice("3 imported into Imported; 4 left out, not channel addresses or too deep"));
    test.core.bookmarks_read(Err("it is not a bookmark file the reader can read".into())).unwrap();
    assert_eq!(test.core.library.manager_notice, notice("Not imported: it is not a bookmark file the reader can read"));
    test.core.bookmarks_read(Ok((vec![link("Only a script", "javascript:void(0)")], 0))).unwrap();
    assert_eq!(test.core.library.manager_notice, notice("No bookmarks in that file that this browser can open"));
}

/// Requirement 20.17: the manager's folders, a new folder, removal and
/// undo, a move refused, an import without its file, an export.
#[test]
fn the_manager_edits_by_number() {
    let mut test = Test::new("manager");
    test.core.session.bookmarks.add(BAR, "News", "news.invalid").unwrap();
    test.core.bookmark_request(BookmarkRequest::Panel(true)).unwrap();
    let view = test.last_bookmarks();
    assert_eq!(view.path.iter().map(|folder| folder.name.as_str()).collect::<Vec<_>>(), ["Bookmarks bar"]);
    assert_eq!(view.items.len(), 1);
    test.core.bookmark_request(BookmarkRequest::NewFolder("Weather".into())).unwrap();
    let view = test.last_bookmarks();
    let (news, weather) = (view.items[0].number, view.items[1].number);
    test.core.bookmark_request(BookmarkRequest::Move { number: news, folder: weather, index: 0 }).unwrap();
    let view = test.last_bookmarks();
    let weather = view.items.iter().find(|item| item.folder).unwrap().number;
    test.core.bookmark_request(BookmarkRequest::Remove(weather)).unwrap();
    let view = test.last_bookmarks();
    assert_eq!(view.undo, "Removed Weather");
    assert_eq!(view.items, Vec::new());
    test.core.bookmark_request(BookmarkRequest::Undo).unwrap();
    assert_eq!(test.last_bookmarks().items.len(), 1);
    test.core.bookmark_request(BookmarkRequest::Import("/nowhere/bookmarks.txt".into())).unwrap();
    assert!(test.last_bookmarks().notice.starts_with("Not imported"));
    test.core.bookmark_request(BookmarkRequest::Export).unwrap();
    let notice = test.last_bookmarks().notice;
    assert!(notice.starts_with("Exported to") && notice.contains("bookmarks-export-"), "{notice}");
    test.core.bookmark_request(BookmarkRequest::Query("news".into())).unwrap();
    let found = test.last_bookmarks();
    assert_eq!((found.items.len(), found.items[0].place.as_str()), (1, "Bookmarks bar / Weather"));
}

/// Requirement 20.18: the bar shows the Bookmarks bar folder when on,
/// and the setting is kept.
#[test]
fn the_bar_shows_its_folder() {
    let mut test = Test::new("bar");
    test.core.session.bookmarks.add(BAR, "News", "news.invalid").unwrap();
    test.core.send_bar().unwrap();
    let hidden = test.sent().into_iter().find_map(|message| if let ToInterface::Bar(view) = message { Some(view) } else { None }).unwrap();
    assert!(!hidden.shown && hidden.items.is_empty());
    test.core.bar_request(&BarRequest::Show(true)).unwrap();
    let shown = test.sent().into_iter().find_map(|message| if let ToInterface::Bar(view) = message { Some(view) } else { None }).unwrap();
    assert_eq!(shown.items.len(), 1);
    assert!(std::fs::read_to_string(test.folder.join("settings.txt")).unwrap().contains("bar on"));
    let _ = test.core.bar_request(&BarRequest::Open { number: shown.items[0].number, new_tab: true });
    assert_eq!(test.core.session.tabs.len(), 1);
    assert_eq!(test.core.session.strip().address, "https://news.invalid/");
}

/// Requirement 20.10: the list and a step of several.
#[test]
fn a_list_entry_goes_there() {
    let mut test = Test::new("list");
    test.core.session.open("a.invalid").unwrap();
    test.core.session.navigate("b.invalid", true).unwrap();
    test.core.session.navigate("c.invalid", true).unwrap();
    test.core.send_tab_list(false).unwrap();
    let list = test.sent().into_iter().find_map(|message| if let ToInterface::TabList(list) = message { Some(list) } else { None }).unwrap();
    assert_eq!(list.entries.iter().map(|entry| entry.title.as_str()).collect::<Vec<_>>(), ["b.invalid", "a.invalid"]);
    let _ = test.core.go_to(false, list.entries[1].steps);
    assert_eq!(test.core.session.strip().identity, "a.invalid");
}
