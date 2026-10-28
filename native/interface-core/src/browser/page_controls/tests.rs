use super::*;
use crate::browser::{Frame, Modifiers, StripState, TabInfo};
use crate::canvas::PageKey;
use crate::channel::{Channel, PAGES, PageInfo, Status};
use crate::test_canvas::Recorder;
use alloc::format;
use alloc::vec;

const WIDTH: f64 = 1440.0;
const HEIGHT: f64 = 900.0;
const OPEN: u64 = 1_000;

/// A browser with a page open, its controls as the trusted core sent
/// them with `subpages` sub-pages.
fn opened(subpages: u16) -> (Browser, PageKey) {
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
    browser.set_tabs(vec![TabInfo { title: "one".into(), colour: 0, channel: 7, muted: false, private: false, address: "https://example.invalid/".into() }], 0);
    browser.set_strip(StripState { address: "https://news.invalid/".into(), identity: "news.invalid".into(), ..StripState::default() });
    let mut channel = Channel::empty();
    channel.channel = 7;
    for index in 0..PAGES {
        channel.pages[index] = Some(PageInfo { title: format!("Page {index}"), status: Status::Loaded });
    }
    browser.set_channel(channel);
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, 0);
    let (key, _) = browser.wall().shown().next().unwrap();
    browser.wall_mut().zoom(key, 0);
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, OPEN);
    browser.set_page_controls(PageControls { page: Some(key.page), subpages, fastext: [true; 5], ..PageControls::default() });
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, OPEN);
    browser.take_outputs();
    (browser, key)
}

/// Decision 67: the engine viewer's keys ask for the open page's
/// controls; the arrows step sub-pages only where there are some.
#[test]
fn the_viewers_keys_ask_for_the_controls() {
    let (mut browser, _) = opened(3);
    for (text, control) in [
        ("r", PageControl::Fastext(FastextKey::Red)),
        ("c", PageControl::Fastext(FastextKey::Cyan)),
        ("i", PageControl::Fastext(FastextKey::Index)),
        ("?", PageControl::Reveal),
        ("H", PageControl::Hold),
        ("m", PageControl::Mix),
    ] {
        browser.key(Key::Text(text), Modifiers::default(), OPEN);
        assert_eq!(browser.take_outputs(), [Output::PageControl(control)], "{text}");
    }
    browser.key(Key::Right, Modifiers::default(), OPEN);
    browser.key(Key::Left, Modifiers::default(), OPEN);
    assert_eq!(browser.take_outputs(), [Output::PageControl(PageControl::SubPage(true)), Output::PageControl(PageControl::SubPage(false))]);
    let (mut single, _) = opened(1);
    single.key(Key::Right, Modifiers::default(), OPEN);
    assert!(single.take_outputs().is_empty(), "one sub-page: the arrow sizes the pages as before");
}
