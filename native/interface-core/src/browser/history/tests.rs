use crate::browser::fixture::{blank_browser, middle};
use alloc::string::String;
use alloc::vec::Vec;

use crate::browser::{Browser, Frame, Key, Modifiers, Output};
use crate::geometry::Point;
use crate::library::{ClearRange, ClosedTab, HistoryRequest, HistoryRow, HistoryView};
use crate::test_canvas::Recorder;

fn row(number: u32, heading: Option<&str>, address: &str) -> HistoryRow {
    HistoryRow {
        number,
        heading: heading.map(Into::into),
        time: "14:05".into(),
        channel: "News Example".into(),
        page: Some(0x302),
        title: "WEATHER".into(),
        address: address.into(),
        // The first row's address is bookmarked.
        bookmarked: number == 7,
    }
}

fn view() -> HistoryView {
    HistoryView {
        query: String::new(),
        first: 0,
        total: 2,
        rows: alloc::vec![row(7, Some("Today: Thursday 22 October 2026"), "https://news.example/#302"), row(8, None, "https://sport.example/")],
        closed: alloc::vec![ClosedTab { number: 3, title: "weather.example".into() }],
        notice: String::new(),
    }
}

fn open() -> Browser {
    let mut browser = blank_browser();
    browser.key(Key::Text("y"), Modifiers { command: true, ..Modifiers::default() }, 0);
    browser.set_history(view());
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    browser.take_outputs();
    browser
}

/// Requirement 20.4: the panel opens by its key, shows the day's heading,
/// each row's title and address, and opens a row by its number.
#[test]
fn the_panel_shows_rows_and_opens_them_by_number() {
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    browser.key(Key::Text("y"), Modifiers { command: true, ..Modifiers::default() }, 0);
    assert_eq!(browser.take_outputs(), [Output::History(HistoryRequest::Panel(true))]);
    browser.set_history(view());
    let mut canvas = Recorder::new();
    browser.draw(&mut canvas, 1440.0, 900.0, 0);
    let texts: Vec<&str> = canvas.texts.iter().map(|(_, text)| text.as_str()).collect();
    for expected in ["History", "Today: Thursday 22 October 2026", "14:05", "News Example 302 WEATHER", "https://news.example/#302", "weather.example"] {
        assert!(texts.contains(&expected), "{expected} in {texts:?}");
    }
    let row = browser.history.controls.rows[1].3;
    assert!(browser.history_press(middle(row), false));
    assert_eq!(browser.take_outputs(), [Output::History(HistoryRequest::Open { number: 8, new_tab: false }), Output::History(HistoryRequest::Panel(false))]);
}

/// The middle button and Command-click open a row in a new tab and leave
/// the panel open; on its check box, its menu or a button, nothing.
#[test]
fn the_middle_button_opens_a_row_in_a_new_tab() {
    let mut browser = open();
    let (_, check, menu, row) = browser.history.controls.rows[1];
    let clear = browser.history.controls.clear;
    for (pressed, expected) in [(row, true), (check, false), (menu, false), (clear, false)] {
        let at = if pressed == row { Point::new(row.x0 + 200.0, middle(row).y) } else { middle(pressed) };
        browser.pointer_moved(at.x, at.y, 0);
        assert_eq!(browser.middle_pressed(), expected);
    }
    assert!(!browser.new_tab_pressed(), "the pointer is on Clear history");
    assert_eq!(browser.take_outputs(), [Output::History(HistoryRequest::Open { number: 8, new_tab: true })]);
    assert!(browser.history.open);
}

/// Requirement 20.6: remove a row, selected rows, a channel, a range;
/// reopen a closed tab.
#[test]
fn rows_are_removed_by_number() {
    let mut browser = open();
    let (number, _, menu, _) = browser.history.controls.rows[0];
    // A row not bookmarked has no third item.
    let other = browser.history.controls.rows[1];
    browser.history_press(middle(other.2), false);
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    assert_eq!(browser.history.controls.menu.len(), 2);
    // A press inside the panel but on no item closes the menu.
    browser.history_press(middle(other.2), false);
    assert!(browser.take_outputs().is_empty() && browser.history.menu.is_none());
    browser.pointer_moved(middle(menu).x, middle(menu).y, 0);
    // The row's menu, as Chrome's: more from this site, remove, and,
    // its address being bookmarked, remove the bookmark.
    assert_eq!(number, 7, "the bookmarked row");
    for (index, expected) in
        [(0, HistoryRequest::MoreFromSite(number)), (1, HistoryRequest::Remove(alloc::vec![number])), (2, HistoryRequest::RemoveBookmark(number))]
    {
        browser.history_press(middle(menu), false);
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        let item = browser.history.controls.menu[index].1;
        browser.history_press(middle(item), false);
        assert_eq!(browser.take_outputs(), [Output::History(expected)]);
    }
    // The rows move as the toolbar comes and goes: measured afresh.
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    let check = browser.history.controls.rows[0].1;
    browser.history_press(middle(check), false);
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    let remove_selected = browser.history.controls.remove_selected;
    browser.history_press(middle(remove_selected), false);
    assert_eq!(browser.take_outputs(), [Output::History(HistoryRequest::Remove(alloc::vec![number]))]);
    let clear = browser.history.controls.clear;
    browser.history_press(middle(clear), false);
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    let all = browser.history.controls.ranges.iter().find(|(range, _)| *range == ClearRange::All).unwrap().1;
    browser.history_press(middle(all), false);
    assert_eq!(browser.take_outputs(), [Output::History(HistoryRequest::Clear(ClearRange::All))]);
    // Requirement 21.8: cookies too, or cookies alone.
    browser.history_press(middle(clear), false);
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    let (with_history, with_cookies) = (browser.history.controls.with_history, browser.history.controls.with_cookies);
    browser.history_press(middle(with_cookies), false);
    let hour = browser.history.controls.ranges.iter().find(|(range, _)| *range == ClearRange::Hour).unwrap().1;
    browser.history_press(middle(hour), false);
    assert_eq!(
        browser.take_outputs(),
        [Output::History(HistoryRequest::Clear(ClearRange::Hour)), Output::History(HistoryRequest::ClearCookies(ClearRange::Hour))]
    );
    browser.history_press(middle(clear), false);
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    browser.history_press(middle(with_history), false);
    let day = browser.history.controls.ranges.iter().find(|(range, _)| *range == ClearRange::Day).unwrap().1;
    browser.history_press(middle(day), false);
    assert_eq!(browser.take_outputs(), [Output::History(HistoryRequest::ClearCookies(ClearRange::Day))]);
    let closed = browser.history.controls.closed[0].1;
    browser.history_press(middle(closed), false);
    assert_eq!(browser.take_outputs(), [Output::History(HistoryRequest::Reopen(Some(3)))]);
}

/// The search follows what is typed; the wheel asks for further rows.
#[test]
fn the_search_and_the_wheel_ask_for_rows() {
    let mut browser = open();
    let search = browser.history.controls.search;
    browser.history_press(middle(search), false);
    browser.key(Key::Text("n"), Modifiers::default(), 0);
    assert_eq!(browser.take_outputs(), [Output::TextInput(true), Output::History(HistoryRequest::Query { text: "n".into(), first: 0 })]);
    let mut many = view();
    many.total = 500;
    browser.set_history(many);
    let list = browser.history.controls.list;
    browser.pointer_moved(middle(list).x, middle(list).y, 0);
    assert!(browser.history_scroll(1.0));
    assert_eq!(browser.take_outputs(), [Output::History(HistoryRequest::Query { text: "n".into(), first: 3 })]);
}
