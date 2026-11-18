use super::resize::{MIN_PAGES_HEIGHT, MINIMISED_HEIGHT};
use super::*;
use alloc::format;
use alloc::vec::Vec;

use crate::browser::{Browser, Frame, Key, Modifiers, Output};
use crate::console::{ConsoleEntry, ConsoleTab, ConsoleUpdate, KEPT, Level, Stat};
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
        stats: None,
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
        stats: None,
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
    let update = |text: &str| ConsoleUpdate { stats: None, channel: 1, replace: false, entries: alloc::vec![entry(ConsoleTab::Network, text)] };
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
    browser.console_update(ConsoleUpdate { stats: None, channel: 2, replace: true, entries: alloc::vec![entry(ConsoleTab::Network, "other")] });
    let shown = texts(&mut browser);
    assert!(shown.contains(&"other".into()) && !shown.contains(&"third".into()));
    for n in 0..KEPT + 5 {
        browser.console_update(ConsoleUpdate { stats: None, channel: 2, replace: false, entries: alloc::vec![entry(ConsoleTab::Pages, &format!("{n}"))] });
    }
    assert_eq!(browser.console.entries[ConsoleTab::Pages.index()].len(), KEPT);
    let close = browser.console.close;
    press(&mut browser, close);
    assert_eq!(browser.take_outputs(), [Output::Console(false)]);
    assert!(!browser.console_open());
}

#[test]
fn its_shortcut_on_each_platform() {
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: true });
    browser.key(Key::Text("I"), Modifiers { command: true, shift: true, ..Modifiers::default() }, 0);
    assert_eq!(browser.take_outputs(), [Output::Console(true)], "Control-Shift-I");
    browser.key(Key::Text("i"), Modifiers { command: true, alt: true, ..Modifiers::default() }, 0);
    assert_eq!(browser.take_outputs(), [Output::Console(false)], "Command-Option-I");
}

/// The owner, 27 October 2026: the console is resized with a horizontal
/// grabber. Pulled up it grows, pulled down it shrinks, never past its
/// bounds, and the pointer takes the row-resize shape over it.
#[test]
fn the_grabber_pulls_the_console_taller_and_shorter_within_its_bounds() {
    let mut browser = open();
    texts(&mut browser);
    let before = browser.console.area;
    let grabber = browser.regions.console_grabber;
    let middle = f64::midpoint(grabber.x0, grabber.x1);
    browser.pointer_moved(middle, f64::midpoint(grabber.y0, grabber.y1), 0);
    assert_eq!(browser.cursor(), crate::browser::pointer::Cursor::ResizeRow);
    browser.pointer_pressed(&mut Recorder::new(), false, 0);
    browser.pointer_moved(middle, f64::midpoint(grabber.y0, grabber.y1) - 100.0, 0);
    browser.pointer_released();
    texts(&mut browser);
    assert!((browser.console.area.height() - (before.height() + 100.0)).abs() < 1.0, "100 taller: {:?} from {:?}", browser.console.area, before);
    assert_eq!(browser.console.area.y1, before.y1, "its bottom stays");

    // Pulled far down, it keeps its tab bar and a few lines; far up, it leaves the pages their room.
    for (pull, shortest) in [(10_000.0, true), (-10_000.0, false)] {
        let grabber = browser.regions.console_grabber;
        browser.pointer_moved(middle, f64::midpoint(grabber.y0, grabber.y1), 0);
        browser.pointer_pressed(&mut Recorder::new(), false, 0);
        browser.pointer_moved(middle, f64::midpoint(grabber.y0, grabber.y1) + pull, 0);
        browser.pointer_released();
        texts(&mut browser);
        let height = browser.console.area.height();
        if shortest {
            assert!(height >= BAR_HEIGHT + 4.0 * LINE && height < before.height(), "{height}");
        } else {
            assert!(browser.console.area.y0 > 240.0, "the pages keep their room: {:?}", browser.console.area);
        }
    }
}

/// The owner, 27 October 2026: the Stats tab shows the figures the trusted
/// core sent, each label and its value, keeps them while an update brings
/// none, and has no Pause or Clear.
#[test]
fn the_stats_tab_shows_the_figures_it_was_sent() {
    let mut browser = open();
    let stat = |label: &str, value: &str| Stat { label: label.into(), value: value.into() };
    browser.console_update(ConsoleUpdate {
        stats: Some(alloc::vec![stat("Bytes received", "2,126 bytes"), stat("Input events sent", "17")]),
        channel: 1,
        replace: true,
        entries: Vec::new(),
    });
    browser.console_update(ConsoleUpdate { stats: None, channel: 1, replace: false, entries: alloc::vec![entry(ConsoleTab::Network, "push: connected")] });
    texts(&mut browser);
    let tab = browser.console.tabs.iter().find(|(tab, _)| *tab == Tab::Stats).unwrap().1;
    press(&mut browser, tab);
    let shown = texts(&mut browser);
    for text in ["Bytes received", "2,126 bytes", "Input events sent", "17"] {
        assert!(shown.iter().any(|shown| shown == text), "{text} in {shown:?}");
    }
    assert!(!shown.iter().any(|shown| shown == "Pause" || shown == "Clear"), "{shown:?}");
}

/// Decision 120 (browser issue #3): the minimise button, left of maximise
/// and close, folds the console to its tab bar, as the overview folds to
/// its heading, and gives the pages its room; pressed again, it restores
/// the height the console had.
#[test]
fn the_minimise_button_folds_the_console_to_its_tabs_and_restores_it() {
    let mut browser = open();
    texts(&mut browser);
    let before = browser.console.area;
    let list_bottom = browser.regions.list.y1;
    let (minimise, maximise, close) = (browser.console.minimise, browser.console.maximise, browser.console.close);
    assert!(minimise.x1 < maximise.x0 && maximise.x1 < close.x0, "minimise, maximise, close: {minimise:?} {maximise:?} {close:?}");
    press(&mut browser, minimise);
    let shown = texts(&mut browser);
    let area = browser.console.area;
    assert!((area.height() - (GAP + BAR_HEIGHT + GAP)).abs() < 1e-9, "its tab bar alone: {area:?}");
    assert!((area.y1 - before.y1).abs() < 1e-9, "its bottom stays");
    assert!(shown.contains(&"Network 0".into()) && !shown.contains(&"Nothing yet".into()), "its tabs and no entries: {shown:?}");
    assert!((browser.regions.list.y1 - (list_bottom + before.height() - area.height())).abs() < 1e-9, "the pages take its room");
    let minimise = browser.console.minimise;
    press(&mut browser, minimise);
    texts(&mut browser);
    assert_eq!(browser.console.area, before, "restored");
}

/// Decision 120: the maximise button gives the console the body's height
/// less the pages' minimum; minimised from there and maximised again, it
/// is as tall again, and pressed once more it restores the height it had.
#[test]
fn the_maximise_button_gives_the_console_all_the_pages_can_spare_and_restores_it() {
    let mut browser = open();
    texts(&mut browser);
    let before = browser.console.area;
    let maximise = browser.console.maximise;
    press(&mut browser, maximise);
    texts(&mut browser);
    let tallest = browser.console.area;
    assert!((tallest.y0 - (browser.body.y0 + MIN_PAGES_HEIGHT)).abs() < 1e-9, "the body less the pages' minimum: {tallest:?}");
    assert!((tallest.y1 - before.y1).abs() < 1e-9, "its bottom stays");
    let minimise = browser.console.minimise;
    press(&mut browser, minimise);
    texts(&mut browser);
    assert!((browser.console.area.height() - MINIMISED_HEIGHT).abs() < 1e-9, "minimised from maximised");
    let maximise = browser.console.maximise;
    press(&mut browser, maximise);
    texts(&mut browser);
    assert_eq!(browser.console.area, tallest, "maximised from minimised");
    let maximise = browser.console.maximise;
    press(&mut browser, maximise);
    texts(&mut browser);
    assert_eq!(browser.console.area, before, "restored");
}

/// Decision 120: the pointer is a hand over the console's minimise,
/// maximise and close buttons, as over the overview's.
#[test]
fn the_hand_shows_over_the_consoles_window_buttons() {
    let mut browser = open();
    texts(&mut browser);
    for button in [browser.console.minimise, browser.console.maximise, browser.console.close] {
        browser.pointer_moved(f64::midpoint(button.x0, button.x1), f64::midpoint(button.y0, button.y1), 0);
        assert_eq!(browser.cursor(), crate::browser::pointer::Cursor::Hand, "{button:?}");
    }
}

/// Decision 120: the grabber, pulled, leaves a minimised or a maximised
/// console from the height it shows; pressed and let go without a pull,
/// it leaves either as it was. The buttons then restore the pulled height.
#[test]
fn the_grabber_leaves_minimised_and_maximised() {
    fn pull(browser: &mut Browser, by: Option<f64>) {
        let grabber = browser.regions.console_grabber;
        let (x, y) = (f64::midpoint(grabber.x0, grabber.x1), f64::midpoint(grabber.y0, grabber.y1));
        browser.pointer_moved(x, y, 0);
        browser.pointer_pressed(&mut Recorder::new(), false, 0);
        if let Some(by) = by {
            browser.pointer_moved(x, y + by, 0);
        }
        browser.pointer_released();
        texts(browser);
    }
    let mut browser = open();
    texts(&mut browser);
    let minimise = browser.console.minimise;
    press(&mut browser, minimise);
    texts(&mut browser);
    pull(&mut browser, None);
    assert!(browser.console.minimised(), "a press alone leaves it minimised");
    // Far enough that the height is above the console's least.
    pull(&mut browser, Some(-150.0));
    assert!(!browser.console.minimised());
    assert!((browser.console.area.height() - (MINIMISED_HEIGHT + 150.0)).abs() < 1e-9, "150 above its tab bar: {:?}", browser.console.area);

    let maximise = browser.console.maximise;
    press(&mut browser, maximise);
    texts(&mut browser);
    let tallest = browser.console.area.height();
    pull(&mut browser, None);
    assert!(browser.console.maximised(), "a press alone leaves it maximised");
    pull(&mut browser, Some(100.0));
    assert!(!browser.console.maximised());
    assert!((browser.console.area.height() - (tallest - 100.0)).abs() < 1e-9, "100 below the tallest: {:?}", browser.console.area);

    let pulled = browser.console.area;
    for _ in 0..2 {
        let maximise = browser.console.maximise;
        press(&mut browser, maximise);
        texts(&mut browser);
    }
    assert_eq!(browser.console.area, pulled, "maximise restores the height pulled");
}

#[test]
fn the_mouse_selects_in_the_search() {
    let mut browser = open();
    let mut canvas = Recorder::new();
    browser.draw(&mut canvas, crate::browser::fixture::WIDTH, crate::browser::fixture::HEIGHT, 0);
    let log = browser.console.tabs.iter().find(|(tab, _)| *tab == Tab::Entries(ConsoleTab::Log)).expect("a tab").1;
    press(&mut browser, log);
    browser.draw(&mut canvas, crate::browser::fixture::WIDTH, crate::browser::fixture::HEIGHT, 0);
    let search = browser.console.search_field;
    crate::browser::field_mouse::tests::a_drag_selects(&mut browser, &mut canvas, search);
}

/// REQ-6406 item 2: the recording's buttons are on the console's top line
/// with its other buttons and ask the trusted core for what the state
/// allows; the console's title says what the recording is doing and how
/// much it has written.
#[test]
fn the_recording_buttons_are_on_the_top_line_and_ask() {
    use crate::console::{RecordingRequest, RecordingState, RecordingView};
    let mut browser = open();
    // A tab whose title is words, not the Log's levels and search.
    browser.console.tab = Tab::Stats;
    let name = alloc::string::String::from("2026-11-16T02-10-11Z-app.neottx.com.ntrec");
    let mut shows = |state: RecordingState, words: &str, requests: &[RecordingRequest]| {
        browser.set_recording(RecordingView { state, bytes: 12_345, name: name.clone() });
        let drawn = texts(&mut browser);
        assert!(drawn.iter().any(|text| text == words), "{state:?}: {words:?} among {drawn:?}");
        let places = [browser.console.record_toggle, browser.console.record_stop];
        assert_eq!(places.iter().filter(|place| **place != crate::geometry::Rect::ZERO).count(), requests.len(), "{state:?}: no other button");
        for request in requests {
            let place = if *request == RecordingRequest::Stop { places[1] } else { places[0] };
            let close = browser.console.close;
            assert!((place.y0, place.y1) == (close.y0, close.y1), "{state:?}: {request:?} is on the top line, as Close is");
            press(&mut browser, place);
            assert_eq!(browser.take_outputs(), [Output::Recording(*request)], "{state:?}: {request:?}");
        }
    };
    shows(RecordingState::Off, "Console", &[RecordingRequest::Record]);
    shows(
        RecordingState::Recording,
        "Recording this tab's traffic: 12,345 bytes in 2026-11-16T02-10-11Z-app.neottx.com.ntrec",
        &[RecordingRequest::Pause, RecordingRequest::Stop],
    );
    shows(
        RecordingState::Paused,
        "Recording paused: 12,345 bytes in 2026-11-16T02-10-11Z-app.neottx.com.ntrec",
        &[RecordingRequest::Resume, RecordingRequest::Stop],
    );
    shows(RecordingState::Stopped, "Recording stopped: 12,345 bytes in 2026-11-16T02-10-11Z-app.neottx.com.ntrec", &[RecordingRequest::Record]);
    shows(RecordingState::Private, "A private tab's traffic is never recorded", &[]);
}

/// The scroll bars drawn in the console: each thin fill of the browser's
/// rule colour, the scroll bar's width across.
fn scroll_bars(browser: &mut Browser) -> Vec<Rect> {
    /// `page_list::draw_scroll_bar`'s width.
    const WIDTH: f64 = 4.0;
    let mut canvas = Recorder::new();
    browser.draw(&mut canvas, 1440.0, 900.0, 0);
    let area = browser.console.area;
    canvas
        .fills
        .iter()
        .filter(|(rect, colour)| {
            *colour == crate::surface::Surface::BROWSER.rule && (rect.width() - WIDTH).abs() < 1e-9 && rect.y0 >= area.y0 && rect.y1 <= area.y1
        })
        .map(|(rect, _)| *rect)
        .collect()
}

/// The owner, 16 November 2026: the developer tools have scroll bars where
/// they scroll. An entry list with more entries than it shows has one at
/// its right edge, as long as the share in view, at the bottom while the
/// newest show and higher once the wheel has gone back; a list that fits
/// has none.
#[test]
fn an_entry_list_has_a_scroll_bar_when_it_holds_more_than_it_shows() {
    let mut browser = open();
    browser.console_update(ConsoleUpdate { stats: None, channel: 1, replace: true, entries: alloc::vec![entry(ConsoleTab::Network, "one")] });
    assert_eq!(scroll_bars(&mut browser), [], "one entry fits: no bar");
    let many: Vec<ConsoleEntry> = (0..200).map(|number| entry(ConsoleTab::Network, &format!("push {number}"))).collect();
    browser.console_update(ConsoleUpdate { stats: None, channel: 1, replace: true, entries: many });
    let bars = scroll_bars(&mut browser);
    let [newest] = bars[..] else { panic!("one bar: {bars:?}") };
    let area = browser.console.area;
    assert!(newest.x1 <= area.x1 && newest.x0 > area.x1 - 40.0, "at the right edge: {newest:?} in {area:?}");
    assert!(newest.height() < area.height() / 2.0, "a share of the list's height");
    browser.pointer_moved(f64::midpoint(area.x0, area.x1), area.y1 - 30.0, 0);
    assert!(browser.console_scroll(-600.0), "the wheel, back through the entries");
    let [older] = scroll_bars(&mut browser)[..] else { panic!("one bar still") };
    assert!(older.y0 < newest.y0, "the bar rises as the view goes back: {older:?} from {newest:?}");
    assert!(texts(&mut browser).iter().all(|text| text != "push 199"), "the newest is out of view");
}

/// The Stats tab scrolls: with more figures than it has room for, the
/// wheel moves through them and a scroll bar shows where the view stands.
#[test]
fn the_stats_tab_scrolls_and_has_a_scroll_bar() {
    let mut browser = open();
    let stats: Vec<Stat> = (0..80).map(|number| Stat { label: format!("Figure {number}"), value: format!("{number}") }).collect();
    browser.console_update(ConsoleUpdate { stats: Some(stats), channel: 1, replace: true, entries: Vec::new() });
    texts(&mut browser);
    let tab = browser.console.tabs.iter().find(|(tab, _)| *tab == Tab::Stats).unwrap().1;
    press(&mut browser, tab);
    let shown = texts(&mut browser);
    assert!(shown.iter().any(|text| text == "Figure 0") && !shown.iter().any(|text| text == "Figure 79"), "the first figures: {shown:?}");
    let [top] = scroll_bars(&mut browser)[..] else { panic!("one bar") };
    let area = browser.console.area;
    browser.pointer_moved(f64::midpoint(area.x0, area.x1), area.y1 - 30.0, 0);
    assert!(browser.console_scroll(100_000.0), "the wheel, down to the end");
    let shown = texts(&mut browser);
    assert!(shown.iter().any(|text| text == "Figure 79") && !shown.iter().any(|text| text == "Figure 0"), "the last figures: {shown:?}");
    let [bottom] = scroll_bars(&mut browser)[..] else { panic!("one bar") };
    assert!(bottom.y0 > top.y0 && (bottom.y1 - (area.y1 - GAP)).abs() < 1.0, "the bar is at the foot: {bottom:?} in {area:?}");
    browser.console_update(ConsoleUpdate {
        stats: Some(alloc::vec![Stat { label: "Only".into(), value: "1".into() }]),
        channel: 1,
        replace: true,
        entries: Vec::new(),
    });
    assert!(texts(&mut browser).iter().any(|text| text == "Only"), "fewer figures are shown from the first");
    assert_eq!(scroll_bars(&mut browser), [], "and need no bar");
}
