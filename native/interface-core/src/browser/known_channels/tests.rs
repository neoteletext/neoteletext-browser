use super::*;
use crate::browser::fixture::{blank_browser, middle};
use crate::test_canvas::Recorder;
use crate::viewer_state::KnownRow;

fn row(number: u32, label: &str, remembered: bool, blocked: bool) -> KnownRow {
    KnownRow { number, label: label.into(), has_key: !blocked, remembered, blocked, created: "Thursday 22 October".into(), used: "today".into() }
}

fn view() -> KnownView {
    KnownView {
        query: String::new(),
        first: 0,
        total: 3,
        rows: alloc::vec![
            row(1, "https://news.example/teletext/", false, false),
            row(2, "http://localhost:8080/dev/", true, false),
            row(3, "https://ads.example/", false, true),
        ],
        can_remember: true,
        notice: String::new(),
    }
}

fn open() -> Browser {
    let mut browser = blank_browser();
    browser.toggle_settings();
    browser.manage_known();
    assert_eq!(browser.take_outputs(), [Output::Known(KnownRequest::Panel(true))], "the settings show the list");
    browser.set_known(view());
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    browser.take_outputs();
    browser
}

/// VS-112: the list names each channel, says whether it is known for the
/// visit or remembered, and asks by number: forget, remember, stop
/// remembering, block and unblock, and forget all once confirmed. No row
/// carries a key: the view has none to carry.
#[test]
fn the_list_shows_each_channel_and_asks_by_number() {
    let mut canvas = Recorder::new();
    let mut browser = open();
    browser.draw(&mut canvas, 1440.0, 900.0, 0);
    let texts: alloc::vec::Vec<&str> = canvas.texts.iter().map(|(_, text)| text.as_str()).collect();
    for expected in ["Channels that know you", "https://news.example/teletext/", "Known for this visit", "Remembered", "Blocked", "Stop remembering", "Unblock"]
    {
        assert!(texts.contains(&expected), "{expected} in {texts:?}");
    }
    let [first, second, third] = browser.known.controls.rows[..] else { panic!("three rows") };
    assert!(third.remember.is_none(), "a blocked channel has no key to remember");
    browser.known_press(middle(first.forget));
    browser.known_press(middle(first.remember.unwrap().0));
    browser.known_press(middle(second.remember.unwrap().0));
    browser.known_press(middle(first.block.0));
    browser.known_press(middle(third.block.0));
    assert_eq!(
        browser.take_outputs(),
        [
            Output::Known(KnownRequest::Forget(1)),
            Output::Known(KnownRequest::Remember { number: 1, remembered: true }),
            Output::Known(KnownRequest::Remember { number: 2, remembered: false }),
            Output::Known(KnownRequest::Block { number: 1, blocked: true }),
            Output::Known(KnownRequest::Block { number: 3, blocked: false }),
        ]
    );
    let all = browser.known.controls.forget_all;
    browser.known_press(middle(all));
    assert_eq!(browser.take_outputs(), [], "forgetting all asks first");
    browser.draw(&mut canvas, 1440.0, 900.0, 0);
    let confirm = browser.known.controls.confirm;
    browser.known_press(middle(confirm));
    assert_eq!(browser.take_outputs(), [Output::Known(KnownRequest::ForgetAll)]);
}

/// VS-112: the search asks for the rows that answer it; closing the list
/// stops its rows, and closing the settings closes it too.
#[test]
fn the_search_asks_and_closing_stops_the_rows() {
    let mut canvas = Recorder::new();
    let mut browser = open();
    let search = browser.known.controls.search;
    browser.known_press(middle(search));
    browser.key(Key::Text("n"), Modifiers::default(), 0);
    assert_eq!(browser.take_outputs(), [Output::TextInput(true), Output::Known(KnownRequest::Query { text: "n".into(), first: 0 })]);
    browser.key(Key::Escape, Modifiers::default(), 0);
    browser.key(Key::Escape, Modifiers::default(), 0);
    assert_eq!(browser.take_outputs(), [Output::TextInput(false), Output::Known(KnownRequest::Panel(false))], "Escape leaves the field, then the list");
    assert!(!browser.known_open() && browser.settings_open());
    browser.manage_known();
    browser.draw(&mut canvas, 1440.0, 900.0, 0);
    browser.toggle_settings();
    assert_eq!(browser.take_outputs(), [Output::Known(KnownRequest::Panel(true)), Output::Known(KnownRequest::Panel(false))]);
    assert!(!browser.known_open());
}
