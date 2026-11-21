//! REQ-7037: the keyboard is the open page's only while its rule table asks
//! for keys or text (the trusted core's `keys`); otherwise page numbers,
//! the Fastext keys, Index, reveal, hold and mix are the browser's own
//! viewer's, and nothing typed goes to the channel. The pointer, the wheel
//! and the badge go by the channel taking input, as before.
//!
//! The interface's window code (`window_input` in
//! `native/interface/src/app/input.rs`) gives a key to `terminal_key` while
//! `keyboard_target` names a page and the browser's modifier is not held,
//! and to the browser's own `key` handling otherwise, with F1 to F4 as the
//! coloured keys (`App::key`). It needs a window, so [`route`] stands in
//! for it here.

use super::*;
use crate::browser::fixture::{HEIGHT, WIDTH};
use crate::browser::{Cursor, Key, Modifiers};
use crate::controls::{FastextKey, PageControl, PageControls};
use alloc::string::String;
use neoteletext_input::keys::code;

const NOW: u64 = 10_000;

/// A key as the viewer presses it on the keyboard.
#[derive(Clone, Copy, Debug)]
enum Pressed {
    /// A key that types `text`.
    Text(&'static str),
    /// F1 to F4.
    Colour(FastextKey),
}

/// The trusted core's typing state for `key`'s page, its table asking for
/// input of some class (`asks`) and for the keyboard (`keys`).
fn told(key: PageKey, asks: bool, keys: bool) -> TypingView {
    TypingView { channel: key.channel, page: key.page, on: true, secure: false, refused: false, cursor: None, round_trip_us: None, wheel: false, asks, keys }
}

/// A browser with the first page of a channel of many pages open, its
/// controls (all five Fastext links) as the trusted core sent them, and
/// the trusted core saying keys go to its channel, with `asks` and `keys`.
fn viewer(asks: bool, keys: bool) -> (Browser, PageKey) {
    let (mut browser, _) = crate::browser::fixture::browser();
    let (key, _) = browser.wall().shown().next().expect("a page on the wall");
    browser.wall_mut().zoom(key, 0);
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, NOW);
    browser.set_page_controls(PageControls { channel: key.channel, page: Some(key.page), subpages: 1, fastext: [true; 5], ..PageControls::default() });
    browser.set_typing(told(key, asks, keys), NOW);
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, NOW);
    browser.take_outputs();
    assert_eq!(browser.opened(), Some(key));
    (browser, key)
}

/// What the interface's window code does with a key press (see the
/// module's note).
fn route(browser: &mut Browser, pressed: Pressed) {
    if browser.keyboard_target().is_some() {
        match pressed {
            Pressed::Text(text) => {
                let character = text.chars().next().expect("a character");
                browser.terminal_key(Action::Press, KeyCode::Character(character.to_ascii_lowercase()), 0, Some(text), None);
            }
            Pressed::Colour(colour) => {
                let name = match colour {
                    FastextKey::Red => "F1",
                    FastextKey::Green => "F2",
                    FastextKey::Yellow => "F3",
                    FastextKey::Cyan | FastextKey::Index => "F4",
                };
                browser.terminal_key(Action::Press, KeyCode::Functional(code(name).expect("a key name")), 0, None, None);
            }
        }
        return;
    }
    match pressed {
        Pressed::Text(text) => {
            browser.key(Key::Text(text), Modifiers::default(), NOW);
        }
        Pressed::Colour(colour) => {
            browser.fastext_key(colour);
        }
    }
}

/// The viewer's keys and the control each asks for (decision 67,
/// REQ-756, REQ-890): r, g, y, c and i, reveal, hold and mix.
const VIEWER_KEYS: [(&str, PageControl); 8] = [
    ("r", PageControl::Fastext(FastextKey::Red)),
    ("g", PageControl::Fastext(FastextKey::Green)),
    ("y", PageControl::Fastext(FastextKey::Yellow)),
    ("c", PageControl::Fastext(FastextKey::Cyan)),
    ("i", PageControl::Fastext(FastextKey::Index)),
    ("?", PageControl::Reveal),
    ("H", PageControl::Hold),
    ("m", PageControl::Mix),
];

/// F1 to F4 and the coloured key each is.
const COLOUR_KEYS: [FastextKey; 4] = [FastextKey::Red, FastextKey::Green, FastextKey::Yellow, FastextKey::Cyan];

/// The page the digits 1, 0 and 3 select, which the fixture's channel has.
const TYPED_PAGE: u16 = 0x103;

/// REQ-7037 while the keyboard is the viewer's: every viewer key does its
/// viewer's action, the digits select a page, and nothing reaches the
/// channel; the terminal's entry points send nothing either.
fn the_viewer_has_the_keyboard(browser: &mut Browser, key: PageKey, what: &str) {
    assert_eq!(browser.keyboard_target(), None, "{what}: the keyboard is not the page's");
    assert_eq!(browser.typing_target(), Some(key), "{what}: keys still go to the channel, for the mouse and the badge");
    assert!(!browser.composes_for_channel(), "{what}: the input method composes for the browser");
    for (text, control) in VIEWER_KEYS {
        route(browser, Pressed::Text(text));
        assert_eq!(browser.take_outputs(), [Output::PageControl(control)], "{what}: {text} is the viewer's");
    }
    for colour in COLOUR_KEYS {
        route(browser, Pressed::Colour(colour));
        assert_eq!(browser.take_outputs(), [Output::PageControl(PageControl::Fastext(colour))], "{what}: {colour:?}'s function key");
    }
    // What the interface's other paths would send, called as they are.
    browser.terminal_key(Action::Press, KeyCode::Character('a'), 0, Some("a"), None);
    browser.terminal_key(Action::Release, KeyCode::Character('a'), 0, None, None);
    browser.terminal_text("typed");
    assert!(!browser.terminal_paste("pasted"), "{what}: no paste goes");
    assert!(!browser.terminal_compose(1, "に"), "{what}: no composition goes");
    assert_eq!(intents(&browser.take_outputs()), [] as [Intent; 0], "{what}: nothing typed reaches the channel");
    // The digits last, since they open another page.
    for digit in ["1", "0", "3"] {
        route(browser, Pressed::Text(digit));
    }
    let outputs = browser.take_outputs();
    assert_eq!(intents(&outputs), [] as [Intent; 0], "{what}: no digit reaches the channel");
    assert_eq!(browser.opened(), Some(PageKey { page: TYPED_PAGE, ..key }), "{what}: the digits selected page 103");
}

/// REQ-7037, REQ-756, REQ-890, REQ-6523: on a page of a channel that takes
/// input but has no rule table, the keyboard is the viewer's: digits
/// select a page, r, g, y, c, F1 to F4 and i follow the Fastext links, and
/// reveal, hold and mix work, and no key, text, paste or composition goes
/// to the channel. The badge still says keys go to the channel. On the
/// code before, every key went to the channel, which dropped it.
#[test]
fn a_page_with_no_table_leaves_the_keyboard_to_the_viewer() {
    let (mut browser, key) = viewer(false, false);
    let mut canvas = Recorder::new();
    browser.draw_typing_badge(&mut canvas, Rect::new(0.0, 0.0, 900.0, 40.0));
    let said: String = canvas.texts.iter().map(|(_, text)| text.as_str()).collect();
    assert!(said.contains("Keys go to"), "the badge is unchanged: {said}");
    the_viewer_has_the_keyboard(&mut browser, key, "no table");
}

/// REQ-7037: a table that asks only for pointer presses, moves or the
/// wheel (`asks` true, `keys` false) leaves the keyboard to the viewer as a
/// page with no table does, and the wheel over its picture stays the
/// page's (REQ-6632).
#[test]
fn a_table_asking_only_for_the_pointer_leaves_the_keyboard_to_the_viewer() {
    let (mut browser, key) = viewer(true, false);
    pointer_at(&mut browser, 20.0, 12.0);
    browser.take_outputs();
    assert_eq!(browser.terminal_wheel(0.0, 1.0), Some(false), "the wheel over the picture is the page's");
    assert!(matches!(intents(&browser.take_outputs()).as_slice(), [Intent::Wheel { .. }]));
    assert_eq!(browser.cursor(), Cursor::Arrow, "and no wheel indicator");
    the_viewer_has_the_keyboard(&mut browser, key, "a pointer table");
}

/// REQ-7037, REQ-1378: a table asking for keys (or text, which the trusted
/// core also tells as `keys`) takes every key: each goes to the channel as
/// a key, and the viewer does not act on it, so no control is asked for
/// and the digits select no page.
#[test]
fn a_table_asking_for_the_keyboard_takes_every_key() {
    let (mut browser, key) = viewer(true, true);
    assert_eq!(browser.keyboard_target(), Some(key));
    assert!(browser.composes_for_channel());
    for (text, _) in VIEWER_KEYS {
        route(&mut browser, Pressed::Text(text));
        let outputs = browser.take_outputs();
        assert!(!outputs.iter().any(|output| matches!(output, Output::PageControl(_))), "{text}: the viewer does not act: {outputs:?}");
        assert!(
            matches!(intents(&outputs).as_slice(), [Intent::Key { action: Action::Press, text: Some(sent), .. }] if sent == text),
            "{text} goes to the channel: {outputs:?}"
        );
    }
    for colour in COLOUR_KEYS {
        route(&mut browser, Pressed::Colour(colour));
        let outputs = browser.take_outputs();
        assert!(matches!(intents(&outputs).as_slice(), [Intent::Key { key: KeyCode::Functional(_), .. }]), "{colour:?}'s function key: {outputs:?}");
        assert!(!outputs.iter().any(|output| matches!(output, Output::PageControl(_))), "{colour:?}: {outputs:?}");
    }
    for digit in ["1", "0", "3"] {
        route(&mut browser, Pressed::Text(digit));
    }
    assert_eq!(intents(&browser.take_outputs()).len(), 3, "each digit a key for the channel");
    assert_eq!(browser.opened(), Some(key), "no page selected by the digits");
    browser.terminal_text("typed");
    assert!(browser.terminal_paste("pasted"));
    assert!(browser.terminal_compose(1, "に"));
    assert_eq!(
        intents(&browser.take_outputs()),
        [Intent::Text("typed".into()), Intent::Paste("pasted".into()), Intent::Compose { cursor: 1, text: "に".into() }]
    );
}

/// TODO-7038, the gap REQ-7037 leaves for the owner: the trusted core
/// tells the interface only whether the table asks for keys at all, so a
/// table that names one key (the music player's piano keys, say) is given
/// every key, and a page number or coloured key it does not name goes to
/// the channel's table, which drops it, and the viewer does not act. This
/// test holds today's behaviour so that a change to it is not unnoticed;
/// when TODO-7038 is decided it is rewritten to the choice.
#[test]
fn a_table_asking_for_some_keys_is_given_them_all_todo_7038() {
    // The trusted core's word for such a table is `keys` (its own test,
    // `a_table_naming_one_key_is_given_every_key`).
    let (mut browser, key) = viewer(true, true);
    for pressed in [Pressed::Text("r"), Pressed::Text("1"), Pressed::Colour(FastextKey::Green)] {
        route(&mut browser, pressed);
        let outputs = browser.take_outputs();
        assert_eq!(intents(&outputs).len(), 1, "{pressed:?} goes to the table: {outputs:?}");
        assert!(!outputs.iter().any(|output| matches!(output, Output::PageControl(_))), "{pressed:?}: the viewer does not act");
    }
    assert_eq!(browser.opened(), Some(key));
}

/// REQ-7037, REQ-6632: whether the table asks for the keyboard changes
/// neither whose the wheel is over the picture nor the pointer's shape:
/// with the same `asks`, `keys` false and true answer alike.
#[test]
fn the_keyboard_changes_neither_the_wheels_owner_nor_its_indicator() {
    for asks in [false, true] {
        let answers: Vec<(Option<bool>, Cursor, usize)> = [false, true]
            .into_iter()
            .map(|keys| {
                let (mut browser, _) = viewer(asks, keys);
                pointer_at(&mut browser, 20.0, 12.0);
                browser.take_outputs();
                let cursor = browser.cursor();
                let owner = browser.terminal_wheel(0.0, 1.0);
                let wheels = intents(&browser.take_outputs()).iter().filter(|intent| matches!(intent, Intent::Wheel { .. })).count();
                (owner, cursor, wheels)
            })
            .collect();
        assert_eq!(answers[0], answers[1], "asks {asks}: keys false and true");
        let (owner, cursor, _) = answers[0];
        if asks {
            assert_eq!((owner, cursor), (Some(false), Cursor::Arrow), "the page's wheel");
        } else {
            assert_eq!(owner, None, "the browser's wheel");
            assert!(matches!(cursor, Cursor::Wheel(_)), "the indicator: {cursor:?}");
        }
    }
}

/// REQ-7037, REQ-2944, REQ-6390: the strip's popovers and panels and the
/// address field take the keyboard whether or not the table asks for it,
/// and the browser's own modifier chords step the page list either way,
/// sending nothing to the channel.
#[test]
fn the_strip_panels_and_the_browsers_chords_win_in_both_states() {
    for keys in [false, true] {
        let (mut browser, key) = viewer(true, keys);
        browser.set_star(crate::library::StarView { open: true, ..crate::library::StarView::default() });
        assert_eq!((browser.typing_target(), browser.keyboard_target()), (None, None), "keys {keys}: the star's popover");
        browser.set_star(crate::library::StarView::default());
        browser.toggle_history();
        assert_eq!((browser.typing_target(), browser.keyboard_target()), (None, None), "keys {keys}: the history panel");
        browser.toggle_history();
        browser.toggle_menu();
        assert_eq!((browser.typing_target(), browser.keyboard_target()), (None, None), "keys {keys}: the menu");
        browser.toggle_menu();
        browser.editing = true;
        assert_eq!((browser.typing_target(), browser.keyboard_target()), (None, None), "keys {keys}: the address field");
        browser.editing = false;
        assert_eq!(browser.keyboard_target(), keys.then_some(key), "keys {keys}: back to the page");
        browser.take_outputs();
        let pages = browser.bank_pages();
        let command = Modifiers { command: true, shift: true, ..Modifiers::default() };
        assert!(browser.key(Key::Down, command, NOW));
        assert_eq!(browser.selected, Some(pages[1]), "keys {keys}: the modifier with Down steps the page list");
        assert_eq!(intents(&browser.take_outputs()), [] as [Intent; 0], "keys {keys}: nothing to the channel");
    }
}
