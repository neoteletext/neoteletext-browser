use super::*;
use crate::browser::fixture::browser;
use crate::browser::{Modifiers, Output, PhoneStatus, PhoneUiView};
use crate::canvas::PageKey;
use crate::channel::page_index;
use crate::controls::{FastextKey, PageControl, PageControls};
use crate::test_canvas::Recorder;
use alloc::string::String;
use alloc::vec::Vec;

const WIDTH: f64 = 1440.0;
const HEIGHT: f64 = 900.0;
/// After the zoom's 300 milliseconds: the page is open.
const OPEN: u64 = 1_000;

/// Every key the phone's terminal layout has (the engine's qrkeyboard
/// `Layout::Terminal`), and the keys a hostile keyboard process might name.
const EVERY_KEY: [&str; 16] = [
    "Enter",
    "Tab",
    "Escape",
    "Backspace",
    "Delete",
    "Home",
    "End",
    "ArrowLeft",
    "ArrowRight",
    "ArrowUp",
    "ArrowDown",
    "ColorF0Red",
    "ColorF1Green",
    "ColorF2Yellow",
    "ColorF3Blue",
    "ChannelUp",
];

/// A browser with a paired phone and its first page zoomed and open.
fn opened() -> (Browser, PageKey) {
    let (mut browser, mut canvas) = browser();
    browser.set_phone(PhoneUiView { status: PhoneStatus::Connected, qr: None, pin: None });
    let (key, _) = browser.wall().shown().next().unwrap();
    browser.wall_mut().zoom(key, 0);
    browser.draw(&mut canvas, WIDTH, HEIGHT, OPEN);
    browser.take_outputs();
    (browser, key)
}

/// Presses and releases every key, and types `text`, on the phone.
fn press_everything(browser: &mut Browser, text: &str, now: u64) {
    for name in EVERY_KEY.iter().copied().chain(["a", "1", "/"]) {
        browser.phone_key(true, name, now);
        browser.phone_key(false, name, now);
    }
    browser.phone_text(text, now);
}

/// Decision 99 (finding F-20): with the address field being edited, the
/// phone types nothing into it and moves nothing.
#[test]
fn a_phones_keys_never_reach_the_address_field() {
    let (mut browser, _) = opened();
    browser.key(Key::Text("l"), Modifiers { command: true, ..Modifiers::default() }, OPEN);
    assert!(browser.editing);
    let before = String::from(browser.editor.text());
    let selected = browser.selected;
    browser.take_outputs();
    press_everything(&mut browser, "evil.invalid", OPEN);
    assert_eq!(browser.editor.text(), before, "the address is as it was");
    assert!(browser.editing, "the field still has the keys");
    assert_eq!(browser.selected, selected);
    assert_eq!(browser.take_outputs(), [], "no navigation, no suggestion asked for");
}

/// Decision 99: while a panel of the strip is open the phone's keys are
/// dropped, never the panel's.
#[test]
fn a_phones_keys_never_reach_a_panel() {
    let (mut browser, _) = opened();
    browser.toggle_settings();
    browser.take_outputs();
    let selected = browser.selected;
    press_everything(&mut browser, "401", OPEN);
    assert!(browser.settings_open());
    assert_eq!(browser.selected, selected);
    assert_eq!(browser.take_outputs(), []);
}

/// Decision 99: the page-navigation keys move between pages: three digits
/// choose one, as typed on the phone's keys or in its text box; the arrows
/// step; the colour keys are the open page's Fastext keys.
#[test]
fn a_phones_page_keys_move_between_pages() {
    let (mut browser, mut canvas) = browser();
    for digit in ["4", "0", "1"] {
        browser.phone_key(true, digit, 0);
    }
    assert_eq!(browser.selected, page_index(0x401));
    browser.phone_text("1x0\u{200B}0", 0);
    assert_eq!(browser.selected, page_index(0x100), "only the text's digits");
    browser.phone_key(true, "ArrowDown", 0);
    assert_eq!(browser.selected, page_index(0x101), "the next page of the bank");
    let (key, _) = browser.wall().shown().next().unwrap();
    browser.wall_mut().zoom(key, 0);
    browser.draw(&mut canvas, WIDTH, HEIGHT, OPEN);
    browser.set_page_controls(PageControls { page: Some(key.page), subpages: 1, fastext: [true; 5], ..PageControls::default() });
    browser.take_outputs();
    for (name, colour) in
        [("ColorF0Red", FastextKey::Red), ("ColorF1Green", FastextKey::Green), ("ColorF2Yellow", FastextKey::Yellow), ("ColorF3Blue", FastextKey::Cyan)]
    {
        assert!(browser.phone_key(true, name, OPEN));
        assert!(!browser.phone_key(false, name, OPEN), "a key-up does nothing");
        assert_eq!(browser.take_outputs(), [Output::PageControl(PageControl::Fastext(colour))], "{name}");
    }
}

/// Decision 99: only the digits, the four arrows and the four colour keys
/// are the page view's; every other key the phone names does nothing.
#[test]
fn only_the_page_navigation_keys_are_the_page_views() {
    let navigation: Vec<&str> = EVERY_KEY.iter().copied().filter(|name| phone_page_key(name).is_some()).collect();
    assert_eq!(navigation, ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "ColorF0Red", "ColorF1Green", "ColorF2Yellow", "ColorF3Blue"]);
    let digits: Vec<char> = (0..=u8::MAX).map(char::from).filter(|character| phone_page_key(character.encode_utf8(&mut [0; 4])).is_some()).collect();
    assert_eq!(digits, ['0', '1', '2', '3', '4', '5', '6', '7', '8', '9']);
    let (mut browser, _) = opened();
    let zoomed = browser.wall().zoomed();
    for name in ["Escape", "Enter", "Tab", "Backspace", "r", "?", "H"] {
        assert!(!browser.phone_key(true, name, OPEN), "{name}");
    }
    assert_eq!(browser.wall().zoomed(), zoomed, "Escape left the page open");
    assert_eq!(browser.take_outputs(), []);
}

/// Decision 99 (finding F-20): the phone never answers the strip's
/// question, nor moves its focus, however long after it was asked.
#[cfg(feature = "links")]
#[test]
fn a_phones_keys_never_answer_the_strips_question() {
    use crate::browser::hyperlinks::PROMPT_DELAY_MILLISECONDS;
    use crate::links::{Prompt, PromptKind};
    let (mut browser, _) = opened();
    browser.set_prompt(Some(Prompt { id: 3, kind: PromptKind::Web, destination: "https://www.example.invalid/".into(), secure: true }), OPEN);
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, OPEN);
    let later = OPEN + PROMPT_DELAY_MILLISECONDS * 4;
    browser.phone_key(true, "Tab", later);
    browser.phone_key(true, "Enter", later);
    browser.phone_key(true, "Escape", later);
    press_everything(&mut browser, "1", later);
    assert_eq!(browser.take_outputs(), [], "no Confirm, no Cancel");
    assert_eq!(browser.links.prompt_focus, None, "the question's focus did not move");
    // The window's own keyboard still answers it.
    browser.key(Key::Tab, Modifiers::default(), later);
    browser.key(Key::Enter, Modifiers::default(), later);
    assert_eq!(browser.take_outputs(), [Output::Confirm(3)]);
}
