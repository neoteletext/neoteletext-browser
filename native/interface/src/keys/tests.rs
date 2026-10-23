//! The key reading's tests: the window's keys and the phone's, read as the
//! input system's.

use super::*;
use winit::keyboard::KeyCode as Physical;

fn name_of(code: Physical) -> String {
    format!("{code:?}")
}

fn key(text: &str) -> Key {
    Key::Character(text.into())
}

/// REQ-INT-104: a printable key's text, a Control combination, a named
/// key, located modifiers and the keypad.
#[test]
fn winit_keys_are_the_protocols() {
    let none = ModifiersState::empty();
    let a = parts(Action::Press, &key("a"), &key("a"), PhysicalKey::Code(Physical::KeyA), Some("a"), none).unwrap();
    assert_eq!((a.key, a.mods, a.text.as_deref(), a.base), (KeyCode::Character('a'), 0, Some("a"), None));
    let shifted = parts(Action::Press, &key("A"), &key("a"), PhysicalKey::Code(Physical::KeyA), Some("A"), ModifiersState::SHIFT).unwrap();
    assert_eq!((shifted.key, shifted.mods, shifted.text.as_deref()), (KeyCode::Character('a'), SHIFT, Some("A")));
    let control_c = parts(Action::Press, &key("c"), &key("c"), PhysicalKey::Code(Physical::KeyC), Some("\u{3}"), ModifiersState::CONTROL).unwrap();
    assert_eq!((control_c.key, control_c.mods, control_c.text), (KeyCode::Character('c'), CONTROL, None), "the control character is not text");
    let enter = parts(Action::Press, &Key::Named(NamedKey::Enter), &Key::Named(NamedKey::Enter), PhysicalKey::Code(Physical::Enter), Some("\r"), none).unwrap();
    assert_eq!((enter.key, enter.text), (KeyCode::Functional(2), None));
    let red = parts(
        Action::Press,
        &Key::Named(NamedKey::ColorF0Red),
        &Key::Named(NamedKey::ColorF0Red),
        PhysicalKey::Unidentified(winit::keyboard::NativeKeyCode::Unidentified),
        None,
        none,
    )
    .unwrap();
    assert_eq!(red.key, KeyCode::Functional(92));
    let shift =
        parts(Action::Press, &Key::Named(NamedKey::Shift), &Key::Named(NamedKey::Shift), PhysicalKey::Code(Physical::ShiftRight), None, ModifiersState::SHIFT)
            .unwrap();
    assert_eq!(shift.key, KeyCode::Functional(86), "Right Shift, located");
    let five = parts(Action::Press, &key("5"), &key("5"), PhysicalKey::Code(Physical::Numpad5), Some("5"), none).unwrap();
    assert_eq!((five.key, five.text.as_deref()), (KeyCode::Functional(65), Some("5")), "the keypad's own code, with its text");
    let space = parts(Action::Press, &Key::Named(NamedKey::Space), &Key::Named(NamedKey::Space), PhysicalKey::Code(Physical::Space), Some(" "), none).unwrap();
    assert_eq!((space.key, space.base), (KeyCode::Character(' '), None));
}

/// REQ-INT-116: another layout's letters as typed, `key` from the
/// layout and `base` from the position; AltGr and Option as text; a
/// dead key with no text.
#[test]
fn international_layouts() {
    let none = ModifiersState::empty();
    let russian = parts(Action::Press, &key("й"), &key("й"), PhysicalKey::Code(Physical::KeyQ), Some("й"), none).unwrap();
    assert_eq!((russian.key, russian.text.as_deref(), russian.base), (KeyCode::Character('й'), Some("й"), Some('q')));
    let hebrew = parts(Action::Press, &key("ש"), &key("ש"), PhysicalKey::Code(Physical::KeyA), Some("ש"), none).unwrap();
    assert_eq!((hebrew.text.as_deref(), hebrew.base), (Some("ש"), Some('a')));
    let azerty = parts(Action::Press, &key("a"), &key("a"), PhysicalKey::Code(Physical::KeyQ), Some("a"), none).unwrap();
    assert_eq!((azerty.key, azerty.base), (KeyCode::Character('a'), Some('q')), "AZERTY's a is where Q is");
    let alt_gr = ModifiersState::CONTROL | ModifiersState::ALT;
    let at = parts(Action::Press, &key("@"), &key("q"), PhysicalKey::Code(Physical::KeyQ), Some("@"), alt_gr).unwrap();
    assert_eq!((at.mods, at.text.as_deref()), (0, Some("@")), "German AltGr-Q, as Windows reports it");
    let option_e = parts(Action::Press, &key("´"), &key("e"), PhysicalKey::Code(Physical::KeyE), Some("´"), ModifiersState::ALT).unwrap();
    assert_eq!(option_e.mods, 0, "macOS Option typing another character is a level");
    let alt_x = parts(Action::Press, &key("x"), &key("x"), PhysicalKey::Code(Physical::KeyX), Some("x"), ModifiersState::ALT).unwrap();
    assert_eq!(alt_x.mods, ALT, "Alt typing the key's own letter is a command");
    let dead = parts(Action::Press, &Key::Dead(Some('´')), &key("'"), PhysicalKey::Code(Physical::Quote), None, none).unwrap();
    assert_eq!((dead.key, dead.text), (KeyCode::Functional(keys::code("Dead").unwrap()), None));
    assert_eq!(name_of(Physical::Semicolon), "Semicolon", "winit's names are the W3C's");
}

#[test]
fn command_never_goes() {
    assert_eq!(modifiers(ModifiersState::SUPER | ModifiersState::SHIFT), SHIFT);
    assert_eq!(modifiers(ModifiersState::CONTROL | ModifiersState::ALT), CONTROL | ALT);
}

#[test]
fn a_phones_keys_have_no_modifiers() {
    assert_eq!(phone_parts(true, "Enter").map(|parts| parts.key), Some(KeyCode::Functional(2)));
    assert_eq!(phone_parts(true, "ColorF2Yellow").map(|parts| parts.key), Some(KeyCode::Functional(94)));
    let a = phone_parts(true, "A").unwrap();
    assert_eq!((a.key, a.text.as_deref(), a.mods), (KeyCode::Character('a'), Some("A"), 0));
    assert_eq!(phone_parts(false, "A").and_then(|parts| parts.text), None, "no text on release");
    assert_eq!(phone_parts(true, "NoSuchKey"), None);
}

/// Decision 99: the keys the trusted core passes on while no page is open
/// for typing, protocol's list, are exactly those the page view acts on,
/// among every W3C name and every character a phone may send.
#[cfg(feature = "phone-keyboard")]
#[test]
fn the_cores_page_keys_are_the_page_views() {
    use interface_core::browser::phone_page_key;
    use protocol::keyboard::{PAGE_NAVIGATION_KEYS, navigates_pages};
    let characters: Vec<String> = (0..=0x2FF_u32).filter_map(char::from_u32).map(String::from).collect();
    for name in neoteletext_input::names::KEY_NAMES.iter().copied().chain(characters.iter().map(String::as_str)) {
        assert_eq!(phone_page_key(name).is_some(), navigates_pages(name), "{name:?}");
    }
    assert!(PAGE_NAVIGATION_KEYS.iter().all(|name| phone_page_key(name).is_some()));
}
