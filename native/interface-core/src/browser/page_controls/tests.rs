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
    browser.set_tabs(
        vec![TabInfo { title: "one".into(), colour: 0, channel: 7, muted: false, private: false, costly: false, address: "https://example.invalid/".into() }],
        0,
    );
    browser.set_strip(StripState { address: "https://news.invalid/".into(), identity: "news.invalid".into(), ..StripState::default() });
    let mut channel = Channel::empty();
    channel.channel = 7;
    for index in 0..PAGES {
        channel.pages[index] = Some(PageInfo { title: format!("Page {index}"), status: Status::Loaded, subpages: 0 });
    }
    browser.set_channel(channel);
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, 0);
    let (key, _) = browser.wall().shown().next().unwrap();
    browser.wall_mut().zoom(key, 0);
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, OPEN);
    browser.set_page_controls(PageControls { channel: key.channel, page: Some(key.page), subpages, fastext: [true; 5], ..PageControls::default() });
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

/// Renderer parity PAR-32 (DEC-6638 item 6): reveal and mix do not apply to
/// a picture, so their keys ask for nothing on a page drawn in ReGIS or
/// NAPLPS, while hold and the coloured keys still do.
#[test]
fn reveal_and_mix_do_nothing_on_a_picture_page() {
    for language in [crate::canvas::PictureLanguage::Regis, crate::canvas::PictureLanguage::Naplps] {
        let (mut browser, key) = opened(1);
        browser.set_regis(key, 0, 1, language);
        browser.take_outputs();
        for text in ["?", "m"] {
            browser.key(Key::Text(text), Modifiers::default(), OPEN);
            assert!(!browser.take_outputs().iter().any(|output| matches!(output, Output::PageControl(_))), "{text} on {language:?}");
        }
        browser.key(Key::Text("H"), Modifiers::default(), OPEN);
        assert_eq!(browser.take_outputs(), [Output::PageControl(PageControl::Hold)], "hold applies to pictures");
    }
}
