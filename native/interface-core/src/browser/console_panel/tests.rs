use super::*;
use alloc::format;
use alloc::vec::Vec;

use crate::browser::{Browser, Frame, Key, Modifiers, Output};
use crate::console::{ConsoleEntry, ConsoleTab, ConsoleUpdate, KEPT, Level};
use crate::geometry::Rect;
use crate::test_canvas::Recorder;
use crate::theme::{RED, YELLOW};

fn entry(tab: ConsoleTab, text: &str) -> ConsoleEntry {
    ConsoleEntry { tab, level: Level::Info, at_ms: 1_250, text: text.into() }
}

fn log(level: Level, text: &str) -> ConsoleEntry {
    ConsoleEntry { tab: ConsoleTab::Log, level, at_ms: 2_000, text: text.into() }
}

/// Decision 70: the Log tab shows each line in its level's colour, and
/// its level buttons and search choose which lines show.
#[test]
fn the_log_filters_by_level_and_search() {
    let mut browser = open();
    browser.console_update(ConsoleUpdate {
        channel: 1,
        replace: true,
        entries: alloc::vec![
            log(Level::Error, "decoder: stopped: broken pipe"),
            log(Level::Warning, "fetcher: 404 for 205.tti"),
            log(Level::Info, "fetcher: confined: sandbox-exec"),
        ],
    });
    texts(&mut browser);
    let tab = browser.console.tabs.iter().find(|(tab, _)| *tab == Tab::Entries(ConsoleTab::Log)).unwrap().1;
    press(&mut browser, tab);
    let mut canvas = Recorder::new();
    browser.draw(&mut canvas, 1440.0, 900.0, 0);
    let colour_of = |canvas: &Recorder, text: &str| canvas.coloured_text(text);
    assert_eq!(colour_of(&canvas, "decoder: stopped: broken pipe"), Some(RED));
    assert_eq!(colour_of(&canvas, "fetcher: 404 for 205.tti"), Some(YELLOW));
    let info = browser.console.levels.iter().find(|(level, _)| *level == Level::Info).unwrap().1;
    press(&mut browser, info);
    let shown = texts(&mut browser);
    assert!(!shown.contains(&"fetcher: confined: sandbox-exec".into()) && shown.contains(&"fetcher: 404 for 205.tti".into()), "information hidden");
    let search = browser.console.search_field;
    press(&mut browser, search);
    assert_eq!(browser.take_outputs(), [Output::TextInput(true)]);
    browser.key(Key::Text("DECODER"), Modifiers::default(), 0);
    let shown = texts(&mut browser);
    assert!(shown.contains(&"decoder: stopped: broken pipe".into()) && !shown.contains(&"fetcher: 404 for 205.tti".into()), "searched, in any case");
    browser.key(Key::Escape, Modifiers::default(), 0);
    assert_eq!(browser.take_outputs(), [Output::TextInput(false)]);
}

fn texts(browser: &mut Browser) -> Vec<alloc::string::String> {
    let mut canvas = Recorder::new();
    browser.draw(&mut canvas, 1440.0, 900.0, 0);
    canvas.texts.into_iter().map(|(_, text)| text).collect()
}

fn press(browser: &mut Browser, rect: Rect) {
    browser.pointer_moved(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1), 0);
    browser.pointer_pressed(&mut Recorder::new(), false, 0);
}

fn open() -> Browser {
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
    browser.key(Key::Text("i"), Modifiers { command: true, alt: true, ..Modifiers::default() }, 0);
    assert_eq!(browser.take_outputs(), [Output::Console(true)]);
    browser
}

/// Requirement 19.8: each tab shows its entries, as text.
#[test]
fn each_tab_shows_its_entries() {
    let mut browser = open();
    let hostile = "<script>alert(1)</script> \u{1b}[31m";
    browser.console_update(ConsoleUpdate {
        channel: 1,
        replace: true,
        entries: alloc::vec![
            entry(ConsoleTab::Network, "GET https://a.example/100.tti 200 512 bytes 12 ms"),
            entry(ConsoleTab::Packets, "pushed M1 R05  Hello  48 65"),
            entry(ConsoleTab::Pages, "page 100 arrived: News"),
            entry(ConsoleTab::Log, hostile),
        ],
    });
    let shown = texts(&mut browser);
    assert!(shown.iter().any(|text| text.starts_with("GET https://a.example/")), "{shown:?}");
    assert!(shown.contains(&"1.250 s".into()));
    assert!(!shown.iter().any(|text| text.starts_with("pushed")), "another tab's entries are not shown");
    for (index, expected) in [(1, "pushed M1 R05  Hello  48 65"), (2, "page 100 arrived: News"), (3, hostile)] {
        let tab = browser.console.tabs[index].1;
        press(&mut browser, tab);
        assert!(texts(&mut browser).contains(&expected.into()), "{expected}: shown as the text it is");
    }
}

/// Requirement 19.8: pause stops it scrolling, clear empties it; a
/// channel's entries replace another's.
#[test]
fn pause_holds_and_clear_empties() {
    let mut browser = open();
    texts(&mut browser);
    let update = |text: &str| ConsoleUpdate { channel: 1, replace: false, entries: alloc::vec![entry(ConsoleTab::Network, text)] };
    browser.console_update(update("first"));
    let pause = browser.console.pause;
    press(&mut browser, pause);
    browser.console_update(update("second"));
    let shown = texts(&mut browser);
    assert!(shown.contains(&"first".into()) && !shown.contains(&"second".into()) && shown.contains(&"Resume".into()));
    press(&mut browser, pause);
    assert!(texts(&mut browser).contains(&"second".into()), "resumed");
    let clear = browser.console.clear;
    press(&mut browser, clear);
    assert!(texts(&mut browser).contains(&"Nothing yet".into()));
    browser.console_update(update("third"));
    browser.console_update(ConsoleUpdate { channel: 2, replace: true, entries: alloc::vec![entry(ConsoleTab::Network, "other")] });
    let shown = texts(&mut browser);
    assert!(shown.contains(&"other".into()) && !shown.contains(&"third".into()));
    for n in 0..KEPT + 5 {
        browser.console_update(ConsoleUpdate { channel: 2, replace: false, entries: alloc::vec![entry(ConsoleTab::Pages, &format!("{n}"))] });
    }
    assert_eq!(browser.console.entries[ConsoleTab::Pages.index()].len(), KEPT);
    let close = browser.console.close;
    press(&mut browser, close);
    assert_eq!(browser.take_outputs(), [Output::Console(false)]);
    assert!(!browser.console_open());
}

/// Requirement 19.8 (decision 63): choosing the Cookies tab asks for
/// the cookies' rows; choosing another stops them.
#[test]
fn the_cookies_tab_asks_for_rows_and_another_stops_them() {
    let mut browser = open();
    texts(&mut browser);
    let cookies = browser.console.tabs.iter().find(|(tab, _)| *tab == Tab::Cookies).unwrap().1;
    press(&mut browser, cookies);
    assert_eq!(browser.take_outputs(), [Output::Cookies(crate::cookies::CookiesRequest::Panel(true))]);
    assert!(browser.cookies_open());
    texts(&mut browser);
    let network = browser.console.tabs[0].1;
    press(&mut browser, network);
    assert_eq!(browser.take_outputs(), [Output::Cookies(crate::cookies::CookiesRequest::Panel(false))]);
    assert!(!browser.cookies_open());
}

#[test]
fn its_shortcut_on_each_platform() {
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: true });
    browser.key(Key::Text("I"), Modifiers { command: true, shift: true, ..Modifiers::default() }, 0);
    assert_eq!(browser.take_outputs(), [Output::Console(true)], "Control-Shift-I");
    browser.key(Key::Text("i"), Modifiers { command: true, alt: true, ..Modifiers::default() }, 0);
    assert_eq!(browser.take_outputs(), [Output::Console(false)], "Command-Option-I");
}
