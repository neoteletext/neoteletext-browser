//! The window's keys as the input system has them
//! (`neoteletext-common/INPUT.md` sections 5.5 to 5.7): winit names its
//! keys by the W3C UI Events values, the names the engine's functional
//! codes number, so a key's code is looked up as winit spells it.

use neoteletext_input::event::{ALT, Action, CONTROL, KeyCode, SHIFT, printable};
use neoteletext_input::keys;
use winit::keyboard::{Key, ModifiersState, NamedKey, PhysicalKey};

#[cfg(test)]
mod tests;

/// A key as the trusted core is given it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyParts {
    /// Press, repeat or release.
    pub action: Action,
    /// The unshifted character of the active layout, or a functional code.
    pub key: KeyCode,
    /// The protocol's modifier bits.
    pub mods: u8,
    /// What the key typed, when it typed printable text.
    pub text: Option<String>,
    /// The United States layout's character of the same physical key.
    pub base: Option<char>,
}

/// The W3C name of a physical key, as winit spells it.
fn physical_name(physical: PhysicalKey) -> Option<String> {
    match physical {
        PhysicalKey::Code(code) => Some(format!("{code:?}")),
        PhysicalKey::Unidentified(_) => None,
    }
}

/// The functional code of a named key: the located modifier when the
/// physical key says which (codes 80 to 91), the keypad's own keys, or
/// the name's code.
fn functional(named: NamedKey, physical: PhysicalKey) -> u32 {
    let located = physical_name(physical).and_then(|name| keys::code(&name)).filter(|&code| keys::is_modifier(code));
    let name = format!("{named:?}");
    located.or_else(|| keys::code(&name)).unwrap_or_else(keys::unidentified)
}

/// Whether the physical key is on the keypad: in `ALL_KEYS` its code is the
/// keypad's, its text otherwise (section 5.7 rule 8).
fn keypad(physical: PhysicalKey) -> Option<u32> {
    match physical {
        PhysicalKey::Code(code) if format!("{code:?}").starts_with("Numpad") => keys::code(&format!("{code:?}")),
        _ => None,
    }
}

/// The protocol's modifier bits: Shift, Control and Alt. Command (the
/// Super key) never goes: it is the browser's (INPUT.md section 9.1). The
/// lock states are not known to winit, so they are not sent.
#[must_use]
pub fn modifiers(state: ModifiersState) -> u8 {
    let bit = |set: bool, value: u8| if set { value } else { 0 };
    bit(state.shift_key(), SHIFT) | bit(state.control_key(), CONTROL) | bit(state.alt_key(), ALT)
}

/// A window key as the input system has it. `logical` is the key the
/// layout gives, `unmodified` the same key with no modifier (winit's
/// `key_without_modifiers`), `text` what it typed.
///
/// AltGr and macOS's Option are level shifts, not modifiers (section 5.7
/// rule 5): a key that types printable text with Control and Alt held
/// together, or with Alt alone giving another character than the key's
/// own, is text with neither.
#[must_use]
pub fn parts(action: Action, logical: &Key, unmodified: &Key, physical: PhysicalKey, text: Option<&str>, state: ModifiersState) -> Option<KeyParts> {
    let mut mods = modifiers(state);
    let text = text.filter(|text| !text.is_empty() && text.chars().all(printable)).map(str::to_owned);
    let own = match unmodified {
        Key::Character(own) => {
            let mut characters = own.chars();
            match (characters.next(), characters.next()) {
                (Some(character), None) => Some(character),
                _ => None,
            }
        }
        _ => None,
    };
    if let Some(typed) = &text {
        let control_and_alt = mods & (CONTROL | ALT) == CONTROL | ALT;
        let shifted_by_alt =
            mods & ALT != 0 && mods & CONTROL == 0 && own.is_some_and(|own| !typed.chars().eq(own.to_lowercase()) && !typed.chars().eq(own.to_uppercase()));
        if control_and_alt || shifted_by_alt {
            mods &= !(CONTROL | ALT);
        }
    }
    let key = match logical {
        // winit names Space as a key; the protocol has it as its character.
        Key::Named(NamedKey::Space) => KeyCode::Character(' '),
        Key::Named(named) => KeyCode::Functional(functional(*named, physical)),
        Key::Dead(_) => KeyCode::Functional(keys::code("Dead").unwrap_or_else(keys::unidentified)),
        Key::Unidentified(_) => KeyCode::Functional(keys::unidentified()),
        Key::Character(_) => match keypad(physical) {
            Some(code) => KeyCode::Functional(code),
            // The active layout's character for the key, unshifted.
            None => KeyCode::Character(own?),
        },
    };
    let base = physical_name(physical).and_then(|name| keys::us_base(&name)).filter(|&base| KeyCode::Character(base) != key);
    Some(KeyParts { action, key, mods, text, base })
}

/// A phone's key by its W3C name or its one character (INPUT.md section
/// 10): no modifiers, its text when it is a character.
#[must_use]
pub fn phone_parts(down: bool, name: &str) -> Option<KeyParts> {
    let action = if down { Action::Press } else { Action::Release };
    let mut characters = name.chars();
    let (key, text) = match (characters.next(), characters.next()) {
        (Some(character), None) if printable(character) => {
            (KeyCode::Character(character.to_lowercase().next().unwrap_or(character)), down.then(|| name.to_owned()))
        }
        _ => (KeyCode::Functional(keys::code(name)?), None),
    };
    Some(KeyParts { action, key, mods: 0, text, base: None })
}
