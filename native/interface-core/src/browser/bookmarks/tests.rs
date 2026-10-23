use super::*;
use alloc::string::String;
use alloc::vec::Vec;

use crate::browser::{Browser, Frame, Key, Modifiers, Output};
use crate::geometry::{Point, Rect};
use crate::library::{BookmarkRequest, BookmarksView, FolderRow, ItemRow};
use crate::test_canvas::Recorder;

fn view() -> BookmarksView {
    BookmarksView {
        path: alloc::vec![FolderRow { number: 1, name: "Bookmarks bar".into(), depth: 0 }],
        folders: alloc::vec![
            FolderRow { number: 2, name: "Bookmarks bar".into(), depth: 0 },
            FolderRow { number: 3, name: "Other bookmarks".into(), depth: 0 },
            FolderRow { number: 4, name: "Weather".into(), depth: 1 },
        ],
        folders_first: 0,
        folders_total: 3,
        query: String::new(),
        items: alloc::vec![
            ItemRow { number: 5, folder: false, name: "News Example".into(), address: "https://news.example/".into(), place: String::new() },
            ItemRow { number: 6, folder: true, name: "Sport".into(), address: String::new(), place: String::new() },
        ],
        items_first: 0,
        items_total: 2,
        undo: String::new(),
        notice: String::new(),
    }
}

fn open() -> Browser {
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    browser.key(Key::Text("b"), Modifiers { command: true, alt: true, ..Modifiers::default() }, 0);
    browser.set_bookmarks(view(), 0);
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    browser.take_outputs();
    browser
}

fn middle(rect: Rect) -> Point {
    Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1))
}

fn click(browser: &mut Browser, rect: Rect) -> Vec<Output> {
    let point = middle(rect);
    browser.pointer_moved(point.x, point.y, 0);
    browser.bookmarks_press(point);
    browser.bookmarks_released(false);
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    browser.take_outputs()
}

/// Requirement 20.17: open a bookmark or a folder by number; each
/// shows its address beside its name.
#[test]
fn items_open_by_number() {
    let mut browser = open();
    let mut canvas = Recorder::new();
    browser.draw(&mut canvas, 1440.0, 900.0, 0);
    let texts: Vec<&str> = canvas.texts.iter().map(|(_, text)| text.as_str()).collect();
    for expected in ["Bookmarks", "News Example", "https://news.example/", "Weather"] {
        assert!(texts.contains(&expected), "{expected} in {texts:?}");
    }
    let folder = browser.bookmarks.controls.items[1].row;
    assert_eq!(click(&mut browser, folder), [Output::Bookmarks(BookmarkRequest::Folder(6))]);
    let bookmark = browser.bookmarks.controls.items[0].row;
    assert_eq!(
        click(&mut browser, bookmark),
        [Output::Bookmarks(BookmarkRequest::Open { number: 5, new_tab: false }), Output::Bookmarks(BookmarkRequest::Panel(false))]
    );
}

/// "Move…" opens a chooser of the folders, the item's own left out; a
/// folder chosen asks for the move, to the folder's end; Escape or a
/// press elsewhere closes it and asks for nothing.
#[test]
fn move_to_chooses_a_folder() {
    let mut browser = open();
    let sport = browser.bookmarks.controls.items[1];
    let moving = sport.moving;
    browser.pointer_moved(middle(sport.row).x, middle(sport.row).y, 0);
    let mut canvas = Recorder::new();
    browser.draw(&mut canvas, 1440.0, 900.0, 0);
    assert!(canvas.texts.iter().any(|(_, text)| text == "Move\u{2026}"), "shown on the row under the pointer");
    browser.bookmarks_press(middle(moving));
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    let chosen: Vec<u32> = browser.bookmarks.controls.chooser.iter().map(|&(number, _)| number).collect();
    assert_eq!(chosen, [2, 3, 4]);
    let weather = browser.bookmarks.controls.chooser[2].1;
    assert!(browser.bookmarks_press(middle(weather)));
    assert_eq!(browser.take_outputs(), [Output::Bookmarks(BookmarkRequest::Move { number: 6, folder: 4, index: u32::MAX })]);
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    assert!(browser.bookmarks.controls.chooser.is_empty(), "closed once chosen");
    // Escape closes it; the manager stays open.
    browser.bookmarks_press(middle(moving));
    browser.key(Key::Escape, Modifiers::default(), 0);
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    assert!(browser.bookmarks.controls.chooser.is_empty() && browser.bookmarks.open);
    // A press elsewhere closes it and does nothing else.
    browser.bookmarks_press(middle(moving));
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    let bookmark = browser.bookmarks.controls.items[0].row;
    assert!(browser.bookmarks_press(middle(bookmark)));
    browser.bookmarks_released(false);
    assert_eq!(browser.take_outputs(), []);
    // A folder moved never lists itself.
    let news = browser.bookmarks.controls.items[0].moving;
    browser.set_bookmarks(
        BookmarksView {
            items: alloc::vec![ItemRow { number: 4, folder: true, name: "Weather".into(), address: String::new(), place: String::new() }],
            items_total: 1,
            ..view()
        },
        0,
    );
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    browser.bookmarks_press(middle(news));
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    assert!(browser.bookmarks.controls.chooser.iter().all(|&(number, _)| number != 4));
}

/// The middle button and Command-click open a bookmark in a new tab,
/// and leave the manager open; on a folder, a button or the chooser
/// they open nothing.
#[test]
fn the_middle_button_and_command_click_open_in_a_new_tab() {
    let mut browser = open();
    let [news, sport] = [browser.bookmarks.controls.items[0], browser.bookmarks.controls.items[1]];
    for (pressed, expected) in [(news.row, true), (sport.row, false), (news.remove, false), (news.edit, false)] {
        browser.pointer_moved(middle(pressed).x, middle(pressed).y, 0);
        assert_eq!(browser.middle_pressed(), expected);
    }
    assert_eq!(browser.take_outputs(), [Output::Bookmarks(BookmarkRequest::Open { number: 5, new_tab: true })]);
    browser.pointer_moved(middle(news.row).x, middle(news.row).y, 0);
    assert!(browser.new_tab_pressed());
    assert_eq!(browser.take_outputs(), [Output::Bookmarks(BookmarkRequest::Open { number: 5, new_tab: true })]);
    assert!(browser.bookmarks.open);
}

/// Requirement 20.17: a new folder, an edit, a removal and its undo, a
/// drag onto a folder.
#[test]
fn the_manager_asks_for_each_change() {
    let mut browser = open();
    let new_folder = browser.bookmarks.controls.new_folder;
    click(&mut browser, new_folder);
    browser.key(Key::Text("Weather"), Modifiers::default(), 0);
    browser.key(Key::Enter, Modifiers::default(), 0);
    assert_eq!(browser.take_outputs(), [Output::TextInput(false), Output::Bookmarks(BookmarkRequest::NewFolder("Weather".into()))]);
    let edit = browser.bookmarks.controls.items[0].edit;
    browser.pointer_moved(middle(edit).x, middle(edit).y, 0);
    browser.bookmarks_press(middle(edit));
    browser.take_outputs();
    browser.key(Key::Text("a"), Modifiers { command: true, ..Modifiers::default() }, 0);
    browser.key(Key::Text("News"), Modifiers::default(), 0);
    browser.key(Key::Enter, Modifiers::default(), 0);
    assert_eq!(
        browser.take_outputs(),
        [Output::TextInput(false), Output::Bookmarks(BookmarkRequest::Edit { number: 5, name: "News".into(), address: "https://news.example/".into() })]
    );
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    let remove = browser.bookmarks.controls.items[0].remove;
    browser.pointer_moved(middle(remove).x, middle(remove).y, 0);
    browser.bookmarks_press(middle(remove));
    assert_eq!(browser.take_outputs(), [Output::Bookmarks(BookmarkRequest::Remove(5))]);
    browser.set_bookmarks(BookmarksView { undo: "Removed News".into(), ..view() }, 100);
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 200);
    let undo = browser.bookmarks.controls.undo;
    assert!(browser.bookmarks_press(middle(undo)));
    assert_eq!(browser.take_outputs(), [Output::Bookmarks(BookmarkRequest::Undo)]);
    assert!(!browser.bookmarks_undo_shown(100 + UNDO_MILLISECONDS), "ten seconds");
    // Dragged onto Weather in the tree.
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    let (bookmark, weather) = (browser.bookmarks.controls.items[0].row, browser.bookmarks.controls.folders[2].1);
    browser.pointer_moved(middle(bookmark).x, middle(bookmark).y, 0);
    browser.bookmarks_press(middle(bookmark));
    browser.pointer_moved(middle(weather).x, middle(weather).y, 0);
    browser.bookmarks_released(false);
    assert_eq!(browser.take_outputs(), [Output::Bookmarks(BookmarkRequest::Move { number: 5, folder: 4, index: u32::MAX })]);
}
