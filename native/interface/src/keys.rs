//! The window's keys as the engine's key records name them (the engine's
//! REQ-INT-003): winit names its keys by the W3C UI Events values, the
//! same names the engine's tables number, so a key's name is looked up as
//! winit spells it.

use neoteletext_input::key::{ALT, CONTROL, REPEAT, SHIFT};
use neoteletext_input::names::{character_key, named_key, physical_key};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};

/// The engine's key code of a key: a printable character's own code, or a
/// named key's; `None` for text a key types that is not one character of
/// the Basic Multilingual Plane, which goes as text.
#[must_use]
pub fn code(key: &Key) -> Option<u16> {
    match key {
        // winit names Space as a key; the record has it as its character.
        Key::Named(NamedKey::Space) => character_key(' '),
        Key::Named(named) => named_key(&format!("{named:?}")).or_else(|| named_key("Unidentified")),
        Key::Character(text) => {
            let mut characters = text.chars();
            match (characters.next(), characters.next()) {
                (Some(character), None) => character_key(character),
                _ => None,
            }
        }
        Key::Dead(_) => named_key("Dead"),
        Key::Unidentified(_) => named_key("Unidentified"),
    }
}

/// The engine's physical key code: where the key is, 0 when unknown.
#[must_use]
pub fn physical(key: PhysicalKey) -> u16 {
    match key {
        // winit's "Super" keys are the W3C's "Meta" ones.
        PhysicalKey::Code(KeyCode::SuperLeft) => physical_key("MetaLeft"),
        PhysicalKey::Code(KeyCode::SuperRight) => physical_key("MetaRight"),
        PhysicalKey::Code(code) => physical_key(&format!("{code:?}")),
        PhysicalKey::Unidentified(_) => 0,
    }
}

/// The record's modifier bits: Shift, Control and Alt, and a held key's
/// repeat. Command (the Super key) never goes: it is the browser's.
#[must_use]
pub fn modifiers(state: ModifiersState, repeat: bool) -> u16 {
    let bit = |set: bool, value: u16| if set { value } else { 0 };
    bit(state.shift_key(), SHIFT) | bit(state.control_key(), CONTROL) | bit(state.alt_key(), ALT) | bit(repeat, REPEAT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn winit_names_are_the_engines() {
        assert_eq!(code(&Key::Named(NamedKey::Enter)), named_key("Enter"));
        assert_eq!(code(&Key::Named(NamedKey::ArrowUp)), named_key("ArrowUp"));
        assert_eq!(code(&Key::Named(NamedKey::F12)), named_key("F12"));
        assert_eq!(code(&Key::Named(NamedKey::ColorF0Red)), named_key("ColorF0Red"));
        assert_eq!(code(&Key::Named(NamedKey::Space)), Some(0x20));
        assert_eq!(code(&Key::Character("w".into())), Some(u16::from(b'w')));
        assert_eq!(code(&Key::Character("\u{1F600}".into())), None, "goes as text");
        assert_eq!(physical(PhysicalKey::Code(KeyCode::KeyW)), physical_key("KeyW"));
        assert_eq!(physical(PhysicalKey::Code(KeyCode::Numpad5)), physical_key("Numpad5"));
        assert_eq!(physical(PhysicalKey::Code(KeyCode::SuperLeft)), physical_key("MetaLeft"));
        assert_ne!(physical_key("KeyW"), 0);
    }

    #[test]
    fn command_never_goes() {
        let state = ModifiersState::SUPER | ModifiersState::SHIFT;
        assert_eq!(modifiers(state, false), SHIFT);
        assert_eq!(modifiers(ModifiersState::CONTROL | ModifiersState::ALT, true), CONTROL | ALT | REPEAT);
    }
}
