//! REQ-7037: a key is the open page's only when its rule table asks for
//! it (the trusted core's `keys`, and the keys it names); every other key
//! is the browser's own viewer's: page numbers, the Fastext keys, Index,
//! reveal, hold and mix, and nothing of it goes to the channel. The
//! pointer, the wheel and the badge go by the channel taking input, as
//! before.
//!
//! The interface's window code (`window_input` in
//! `native/interface/src/app/input.rs`) gives a key to `terminal_key` while
//! `key_target` names a page for it and the browser's modifier is not
//! held, and to the browser's own `key` handling otherwise, with F1 to F4 as the
//! coloured keys (`App::key`). It needs a window, so [`route`] stands in
//! for it here.

use super::*;
use crate::browser::fixture::{HEIGHT, WIDTH};
use crate::browser::{AskedKey, AskedKeys, Cursor, Key, KeyNamed, Modifiers};
use crate::controls::{FastextKey, PageControl, PageControls};
use alloc::string::String;
use neoteletext_input::event::{ALT, SHIFT};
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
    // A table that asks for the keyboard asks, here, for every key and for
    // text; a test of a table naming some keys says which after.
    browser.set_asked_keys(if keys { AskedKeys::everything() } else { AskedKeys::default() });
    browser.draw(&mut Recorder::new(), WIDTH, HEIGHT, NOW);
    browser.take_outputs();
    assert_eq!(browser.opened(), Some(key));
    (browser, key)
}

/// What the interface's window code does with a key press (see the
/// module's note).
fn route(browser: &mut Browser, pressed: Pressed) {
    let (key, text) = match pressed {
        Pressed::Text(text) => (KeyCode::Character(text.chars().next().expect("a character").to_ascii_lowercase()), Some(text)),
        Pressed::Colour(colour) => {
            let name = match colour {
                FastextKey::Red => "F1",
                FastextKey::Green => "F2",
                FastextKey::Yellow => "F3",
                FastextKey::Cyan | FastextKey::Index => "F4",
            };
            (KeyCode::Functional(code(name).expect("a key name")), None)
        }
    };
    // REQ-7037: a key the page's table asks for is the page's.
    if browser.key_target(Action::Press, key, 0, text, None).is_some() {
        browser.terminal_key(Action::Press, key, 0, text, None);
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

/// REQ-7037 as TODO-7038 settled it: a table that names some keys is
/// given those and no other. The keys it names go to the channel and the
/// viewer does not act on them; every other key is the viewer's, which
/// acts on it, and nothing of it goes to the channel. Here the table
/// names the character `r`, the physical key `1` and F2, and asks for no
/// text. On the code before, every key went to the table, which dropped
/// the ones it did not name.
#[test]
fn a_table_asking_for_some_keys_is_given_those_and_no_other() {
    let (mut browser, key) = viewer(true, true);
    let named = |key: KeyNamed| AskedKey { key, ..AskedKey::EVERY_KEY };
    browser.set_asked_keys(AskedKeys {
        text: false,
        keys: alloc::vec![named(KeyNamed::Character('r')), named(KeyNamed::Physical('1')), named(KeyNamed::Named(code("F2").expect("a key name")))],
    });
    for pressed in [Pressed::Text("r"), Pressed::Text("1"), Pressed::Colour(FastextKey::Green)] {
        route(&mut browser, pressed);
        let outputs = browser.take_outputs();
        assert_eq!(intents(&outputs).len(), 1, "{pressed:?} goes to the table: {outputs:?}");
        assert!(!outputs.iter().any(|output| matches!(output, Output::PageControl(_))), "{pressed:?}: the viewer does not act");
    }
    assert_eq!(browser.opened(), Some(key));
    for (pressed, control) in [(Pressed::Text("g"), FastextKey::Green), (Pressed::Colour(FastextKey::Red), FastextKey::Red)] {
        route(&mut browser, pressed);
        assert_eq!(browser.take_outputs(), [Output::PageControl(PageControl::Fastext(control))], "{pressed:?} is the viewer's");
    }
    assert!(!browser.terminal_paste("pasted"), "a table that asks for no text is given no paste");
    assert!(!browser.composes_for_channel(), "nor a composition");
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

// The tests below are of REQ-7037 version 2 (DEC-7157, TODO-7158): how each
// key is matched against the keys the open page's table names.

/// A functional key by its name in the input protocol's table.
fn named(name: &str) -> KeyCode {
    KeyCode::Functional(code(name).expect("a key name"))
}

/// A key named under no modifier comparison.
fn asked(key: KeyNamed) -> AskedKey {
    AskedKey { key, ..AskedKey::EVERY_KEY }
}

/// The page open, its table naming `keys` and asking for text or not.
fn asking(text: bool, keys: Vec<AskedKey>) -> (Browser, PageKey) {
    let (mut browser, key) = viewer(true, true);
    browser.set_asked_keys(AskedKeys { text, keys });
    (browser, key)
}

/// Whether a press of `key` with `mods`, typing `text`, on the physical key
/// `base`, is the page's.
fn pages(browser: &Browser, key: KeyCode, mods: u8, text: Option<&str>, base: Option<char>) -> bool {
    browser.key_target(Action::Press, key, mods, text, base).is_some()
}

/// REQ-7037 item 3, RULE-10: a character key is named by its unshifted
/// character, a named key by its functional code, and a physical key by
/// the event's United States layout character when it has one, else by its
/// character: on a French layout the key that types `a` is the physical
/// key `q`, and the key that types `q` is not.
#[test]
fn character_named_and_physical_keys_match_as_rule_10_says() {
    let (browser, key) =
        asking(false, alloc::vec![asked(KeyNamed::Character('a')), asked(KeyNamed::Named(code("F5").unwrap())), asked(KeyNamed::Physical('q'))]);
    let character = |character: char| KeyCode::Character(character);
    assert_eq!(browser.key_target(Action::Press, character('a'), 0, Some("a"), Some('a')), Some(key), "the character a");
    assert!(pages(&browser, character('a'), SHIFT, Some("A"), Some('a')), "the character a, shifted, is still its unshifted character");
    assert!(!pages(&browser, character('b'), 0, Some("b"), Some('b')), "another character");
    assert!(pages(&browser, named("F5"), 0, None, None), "the named key F5");
    assert!(!pages(&browser, named("F6"), 0, None, None), "another named key");
    assert!(pages(&browser, character('z'), 0, Some("z"), Some('q')), "the physical key q, whatever it types");
    assert!(!pages(&browser, character('q'), 0, Some("q"), Some('w')), "the character q on another physical key");
    assert!(pages(&browser, character('q'), 0, Some("q"), None), "with no base, the physical key is its character");
}

/// REQ-7037 item 3, RULE-9: a key named with a modifier mask and value is
/// the page's only when the event's modifiers under the mask equal the
/// value; bits outside the mask do not matter.
#[test]
fn a_modifier_mask_and_value_are_honoured() {
    let shifted_s = AskedKey { key: KeyNamed::Character('s'), mods_mask: SHIFT, mods_value: SHIFT };
    let s_without_control = AskedKey { key: KeyNamed::Character('t'), mods_mask: SHIFT | CONTROL, mods_value: SHIFT };
    let (browser, _) = asking(false, alloc::vec![shifted_s, s_without_control]);
    let s = KeyCode::Character('s');
    assert!(!pages(&browser, s, 0, Some("s"), None), "s alone is not Shift-s");
    assert!(pages(&browser, s, SHIFT, Some("S"), None), "Shift-s");
    assert!(pages(&browser, s, SHIFT | ALT, Some("S"), None), "Alt is outside the mask");
    let t = KeyCode::Character('t');
    assert!(pages(&browser, t, SHIFT, Some("T"), None));
    assert!(!pages(&browser, t, SHIFT | CONTROL, None, None), "Control is in the mask and must be up");
}

/// REQ-7037 items 2 and 3: "any key" under a modifier comparison names
/// every key with those modifiers and no key without them.
#[test]
fn any_key_under_a_modifier_comparison() {
    let (browser, _) = asking(false, alloc::vec![AskedKey { key: KeyNamed::Any, mods_mask: CONTROL, mods_value: CONTROL }]);
    assert!(pages(&browser, KeyCode::Character('x'), CONTROL, None, None));
    assert!(pages(&browser, named("F1"), CONTROL, None, None));
    assert!(!pages(&browser, KeyCode::Character('x'), 0, Some("x"), None), "x alone is the viewer's");
    assert!(!pages(&browser, named("F1"), 0, None, None), "F1 alone is the viewer's coloured key");
}

/// REQ-7037 items 3 and 5: a table that asks for text and names no key is
/// given every character key and every key whose press typed a printable
/// character, and the paste and the composition; a named key that types
/// nothing (an arrow, Escape, F1) stays the viewer's.
#[test]
fn a_table_asking_for_text_is_given_every_key_that_types_and_no_other() {
    let (mut browser, key) = asking(true, alloc::vec![]);
    assert!(pages(&browser, KeyCode::Character('a'), 0, Some("a"), None), "a character key");
    assert!(pages(&browser, KeyCode::Character('1'), 0, None, None), "a character key, whatever it typed");
    assert!(pages(&browser, named("Numpad5"), 0, Some("5"), None), "a named key that typed a printable character");
    for name in ["ArrowUp", "Escape", "F1", "Enter"] {
        let text = (name == "Enter").then_some("\r");
        assert!(!pages(&browser, named(name), 0, text, None), "{name} types nothing printable and is the viewer's");
    }
    assert_eq!(browser.text_target(), Some(key));
    assert!(browser.composes_for_channel());
    assert!(browser.terminal_paste("pasted"));
    browser.terminal_text("typed");
    assert!(browser.terminal_compose(1, "に"));
    assert_eq!(
        intents(&browser.take_outputs()),
        [Intent::Paste("pasted".into()), Intent::Text("typed".into()), Intent::Compose { cursor: 1, text: "に".into() }]
    );
    route(&mut browser, Pressed::Colour(FastextKey::Red));
    assert_eq!(browser.take_outputs(), [Output::PageControl(PageControl::Fastext(FastextKey::Red))], "F1 follows the red link");
}

/// REQ-7037 item 5: a table that names keys and asks for no text is given
/// no paste, no composition, no committed text and no phone text (which
/// the window sends through `terminal_text`), and the input method is not
/// switched on for it.
#[test]
fn a_table_naming_keys_and_no_text_is_given_no_text() {
    let (mut browser, _) = asking(false, alloc::vec![AskedKey::EVERY_KEY]);
    assert!(browser.keyboard_target().is_some(), "the keys are the page's");
    assert_eq!(browser.text_target(), None);
    assert!(!browser.composes_for_channel());
    assert!(!browser.terminal_paste("pasted"));
    assert!(!browser.terminal_compose(1, "に"));
    browser.terminal_text("typed");
    assert_eq!(intents(&browser.take_outputs()), [] as [Intent; 0]);
}

/// REQ-7037 item 3: the repeat and the release of a key whose press the
/// page was given go to the page whatever the modifiers have become; one
/// whose press it was not given, or that came up, does not.
#[test]
fn a_keys_repeat_and_release_follow_its_press_after_the_modifiers_change() {
    let (mut browser, key) = asking(false, alloc::vec![AskedKey { key: KeyNamed::Character('a'), mods_mask: SHIFT, mods_value: 0 }]);
    let a = KeyCode::Character('a');
    assert_eq!(browser.key_target(Action::Repeat, a, SHIFT, None, None), None, "not pressed yet: Shift-a is the viewer's");
    browser.terminal_key(Action::Press, a, 0, Some("a"), None);
    assert_eq!(browser.key_target(Action::Repeat, a, SHIFT, None, None), Some(key), "its repeat with Shift now down");
    assert_eq!(browser.key_target(Action::Release, a, SHIFT, None, None), Some(key), "its release");
    assert_eq!(browser.key_target(Action::Press, a, SHIFT, None, None), None, "a new press is matched afresh");
    browser.terminal_key(Action::Release, a, SHIFT, None, None);
    assert_eq!(browser.key_target(Action::Release, a, SHIFT, None, None), None, "up: no longer followed");
    let sent = intents(&browser.take_outputs());
    assert!(matches!(sent.as_slice(), [Intent::Key { action: Action::Press, .. }, Intent::Key { action: Action::Release, .. }]), "{sent:?}");
}

/// REQ-7037 item 3: the interface follows at most `MAX_HELD_KEYS` keys
/// (sixteen) given to the page and still down; the seventeenth's release
/// under a changed modifier is not followed.
#[test]
fn the_seventeenth_held_key_is_not_followed() {
    use neoteletext_input::held::MAX_HELD_KEYS;
    let (mut browser, key) = asking(false, alloc::vec![AskedKey { key: KeyNamed::Any, mods_mask: SHIFT, mods_value: 0 }]);
    let keys: Vec<KeyCode> = ('a'..='z').take(MAX_HELD_KEYS + 1).map(KeyCode::Character).collect();
    assert_eq!(keys.len(), 17, "MAX_HELD_KEYS is sixteen");
    for &down in &keys {
        browser.terminal_key(Action::Press, down, 0, None, None);
    }
    assert_eq!(intents(&browser.take_outputs()).len(), keys.len(), "every press is the page's");
    assert_eq!(browser.key_target(Action::Release, keys[MAX_HELD_KEYS - 1], SHIFT, None, None), Some(key), "the sixteenth is followed");
    assert_eq!(browser.key_target(Action::Release, keys[MAX_HELD_KEYS], SHIFT, None, None), None, "the seventeenth is not");
}

/// REQ-7037 item 3: the held keys are forgotten when typing goes off and
/// when another page opens.
#[test]
fn the_held_keys_are_forgotten_when_typing_goes_off_or_another_page_opens() {
    let only_unshifted = alloc::vec![AskedKey { key: KeyNamed::Character('a'), mods_mask: SHIFT, mods_value: 0 }];
    let a = KeyCode::Character('a');
    let (mut browser, key) = asking(false, only_unshifted.clone());
    browser.terminal_key(Action::Press, a, 0, Some("a"), None);
    browser.set_typing(TypingView { on: false, ..told(key, true, true) }, NOW);
    browser.set_typing(told(key, true, true), NOW);
    assert_eq!(browser.key_target(Action::Release, a, SHIFT, None, None), None, "typing went off");

    let (mut browser, key) = asking(false, only_unshifted);
    browser.terminal_key(Action::Press, a, 0, Some("a"), None);
    browser.set_typing(told(key, true, true), NOW);
    assert_eq!(browser.key_target(Action::Release, a, SHIFT, None, None), Some(key), "the same page told again keeps it");
    browser.set_typing(told(PageKey { page: key.page + 1, ..key }, true, true), NOW);
    assert_eq!(browser.key_target(Action::Release, a, SHIFT, None, None), None, "another page");
}

/// REQ-7037 items 3 and 4: with a table naming some keys, each of the
/// viewer's keys, the coloured keys and the digits either goes to the page
/// and the viewer does nothing, or the viewer acts and nothing goes to the
/// page; never both, and the page gets exactly the keys named.
#[test]
fn no_key_goes_both_to_the_page_and_to_the_viewer() {
    let (mut browser, key) = asking(
        false,
        alloc::vec![
            asked(KeyNamed::Character('y')),
            asked(KeyNamed::Character('?')),
            asked(KeyNamed::Named(code("F4").unwrap())),
            asked(KeyNamed::Character('5'))
        ],
    );
    let mut pressed: Vec<(Pressed, bool)> = VIEWER_KEYS.iter().map(|&(text, _)| (Pressed::Text(text), matches!(text, "y" | "?"))).collect();
    pressed.extend(COLOUR_KEYS.iter().map(|&colour| (Pressed::Colour(colour), colour == FastextKey::Cyan)));
    pressed.push((Pressed::Text("5"), true));
    for (press, named) in pressed {
        route(&mut browser, press);
        let outputs = browser.take_outputs();
        let to_page = !intents(&outputs).is_empty();
        let by_viewer = outputs.iter().any(|output| !matches!(output, Output::Terminal { .. }));
        assert!(to_page != by_viewer, "{press:?} went one way only: {outputs:?}");
        assert_eq!(to_page, named, "{press:?} goes to the page exactly when named: {outputs:?}");
    }
    assert_eq!(browser.opened(), Some(key), "the digit the table names selected no page");
}

/// REQ-7037 item 7 (REQ-2944, REQ-6390): with a table naming some keys,
/// the strip's popover and the address field still take the keys it
/// names, and the browser's chords stay the browser's.
#[test]
fn the_strip_and_the_chords_win_over_a_table_naming_some_keys() {
    let (mut browser, key) = asking(false, alloc::vec![asked(KeyNamed::Character('r')), asked(KeyNamed::Named(code("ArrowDown").unwrap()))]);
    let r = KeyCode::Character('r');
    assert_eq!(browser.key_target(Action::Press, r, 0, Some("r"), None), Some(key));
    browser.editing = true;
    assert_eq!(browser.key_target(Action::Press, r, 0, Some("r"), None), None, "the address field");
    browser.editing = false;
    browser.set_star(crate::library::StarView { open: true, ..crate::library::StarView::default() });
    assert_eq!(browser.key_target(Action::Press, r, 0, Some("r"), None), None, "the star's popover");
    browser.set_star(crate::library::StarView::default());
    browser.take_outputs();
    let pages = browser.bank_pages();
    assert!(browser.key(Key::Down, Modifiers { command: true, shift: true, ..Modifiers::default() }, NOW));
    assert_eq!(browser.selected, Some(pages[1]), "the browser's chord steps the page list");
    assert_eq!(intents(&browser.take_outputs()), [] as [Intent; 0]);
}
