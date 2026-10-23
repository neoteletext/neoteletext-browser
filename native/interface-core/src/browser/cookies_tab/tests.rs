use super::*;
use crate::browser::{Frame, Modifiers};
use crate::cookies::CookieRow;
use crate::test_canvas::Recorder;

fn row(number: u32, site: bool, label: &str, blocked: bool) -> CookieRow {
    CookieRow { number, site, label: label.into(), count: 2, bytes: 2048, used: "Thursday 22 October".into(), blocked }
}

fn view() -> CookiesView {
    CookiesView {
        query: String::new(),
        first: 0,
        total: 3,
        rows: alloc::vec![row(1, true, "https://news.example", false), row(2, false, "/teletext/", false), row(3, false, "/sport/", true)],
        count: 4,
        bytes: 4096,
        notice: String::new(),
    }
}

fn open() -> Browser {
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    browser.manage_cookies();
    assert_eq!(browser.take_outputs(), [Output::Console(true), Output::Cookies(CookiesRequest::Panel(true))], "the console opens at its cookies");
    browser.set_cookies(view());
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    browser.take_outputs();
    browser
}

fn middle(rect: Rect) -> Point {
    Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1))
}

/// Requirement 21.6: the panel lists sites and channels with their
/// counts, sizes and last use, and asks by number: delete a site, a
/// channel, block and unblock, delete all once confirmed.
#[test]
fn the_panel_lists_and_asks_by_number() {
    let mut canvas = Recorder::new();
    let mut browser = open();
    browser.draw(&mut canvas, 1440.0, 900.0, 0);
    let texts: alloc::vec::Vec<&str> = canvas.texts.iter().map(|(_, text)| text.as_str()).collect();
    for expected in [
        "Cookies",
        "https://news.example",
        "/teletext/",
        "2 \u{00B7} 2 KB",
        "Thursday 22 October",
        "Block",
        "Unblock",
        "4 cookies, 4 KB \u{00B7} rows 1 to 3 of 3",
    ] {
        assert!(texts.contains(&expected), "{expected} in {texts:?}");
    }
    let site = browser.cookies.controls.rows[0];
    assert!(site.block.is_none(), "a site is deleted, not blocked");
    browser.cookies_press(middle(site.delete));
    let channel = browser.cookies.controls.rows[1];
    browser.cookies_press(middle(channel.block.unwrap().0));
    let blocked = browser.cookies.controls.rows[2];
    browser.cookies_press(middle(blocked.block.unwrap().0));
    assert_eq!(
        browser.take_outputs(),
        [
            Output::Cookies(CookiesRequest::Delete(site.number)),
            Output::Cookies(CookiesRequest::Block { number: channel.number, blocked: true }),
            Output::Cookies(CookiesRequest::Block { number: blocked.number, blocked: false }),
        ]
    );
    let all = browser.cookies.controls.delete_all;
    browser.cookies_press(middle(all));
    assert_eq!(browser.take_outputs(), [], "not until confirmed");
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    let confirm = browser.cookies.controls.confirm;
    browser.cookies_press(middle(confirm));
    assert_eq!(browser.take_outputs(), [Output::Cookies(CookiesRequest::DeleteAll)]);
}

/// The search follows what is typed; closing the console stops the
/// rows, and opening it again asks for them.
#[test]
fn the_search_asks_for_rows() {
    let mut browser = open();
    let search = browser.cookies.controls.search;
    browser.cookies_press(middle(search));
    browser.key(Key::Text("n"), Modifiers::default(), 0);
    assert_eq!(browser.take_outputs(), [Output::TextInput(true), Output::Cookies(CookiesRequest::Query { text: "n".into(), first: 0 })]);
    browser.key(Key::Escape, Modifiers::default(), 0);
    assert_eq!(browser.take_outputs(), [Output::TextInput(false)]);
    browser.toggle_console();
    assert_eq!(browser.take_outputs(), [Output::Console(false), Output::Cookies(CookiesRequest::Panel(false))]);
    assert!(!browser.cookies_open());
    browser.toggle_console();
    assert_eq!(browser.take_outputs(), [Output::Console(true), Output::Cookies(CookiesRequest::Panel(true))]);
}
