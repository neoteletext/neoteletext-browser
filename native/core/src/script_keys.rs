//! A script's claims as the keys its page is given (the script API's
//! specification, `neoteletext-common/docs/script-api.md`, section 4.6;
//! REQ-7037; TODO-7294): a manifest names its
//! keys by their W3C UI Events key values, and the interface decides
//! whose a key is from key asks, as it does for a page's rule table. A
//! name becomes an ask only when a script may ever be given that key, and
//! every ask compares the modifiers that make a key the browser's own, so
//! that no manifest can take a chord of the browser's.

use interface_core::browser::scripts::{BROWSER_CHORD_MODIFIER_OF_NAMED_KEYS, BROWSER_CHORD_MODIFIERS, claimable_name};
use neoteletext_input::event::KeyCode;
use protocol::decoder::script::ScriptClaims;
use protocol::rules::{Key, KeyAsk, KeyAsks, KeyKind, MAX_KEY_ASKS};

/// The key a manifest's name is: one character names the key that types
/// it without Shift, as the input protocol's KEY event and RULE-10 name a
/// character key; any longer name is a functional key of the engine's
/// table. None for a name that is no key, and for a key no script is
/// ever given.
fn key_of(name: &str) -> Option<Key> {
    let mut characters = name.chars();
    match (characters.next(), characters.next()) {
        (Some(character), None) if !character.is_control() => Some(Key::of(KeyKind::Character, u32::from(character))),
        (Some(_), Some(_)) if claimable_name(name) => neoteletext_input::keys::code(name).map(|code| Key::of(KeyKind::Named, code)),
        _ => None,
    }
}

/// The key asks of what a channel's scripts claim, and the names that
/// became none: a name no script is ever given, and a name that is no
/// key. There is no "any key" ask (section 4.6).
#[must_use]
pub fn asks_of(claims: &ScriptClaims) -> (KeyAsks, Vec<&str>) {
    let mut keys = Vec::new();
    let mut ignored = Vec::new();
    for name in claims.keys.iter().take(MAX_KEY_ASKS) {
        let Some(key) = key_of(name) else {
            ignored.push(name.as_str());
            continue;
        };
        // Alt with a key that types nothing is the browser's or the
        // system's; with a character key it types.
        let mask = if key.kind == KeyKind::Named { BROWSER_CHORD_MODIFIERS | BROWSER_CHORD_MODIFIER_OF_NAMED_KEYS } else { BROWSER_CHORD_MODIFIERS };
        let ask = KeyAsk { key: Some(key), mods_mask: mask, mods_value: 0 };
        if !keys.contains(&ask) {
            keys.push(ask);
        }
    }
    (KeyAsks { text: claims.text, keys }, ignored)
}

/// Whether `asks` names the key of an event, as the interface matches it
/// (RULE-9, RULE-10): the key, and the event's modifiers under the ask's
/// mask equal to its value.
#[must_use]
pub fn names(asks: &KeyAsks, key: KeyCode, mods: u8) -> bool {
    asks.keys.iter().any(|ask| {
        let named = match (ask.key, key) {
            (Some(named), KeyCode::Character(character)) => named.kind == KeyKind::Character && named.code == u32::from(character),
            (Some(named), KeyCode::Functional(code)) => named.kind == KeyKind::Named && named.code == code,
            (None, _) => false,
        };
        named && mods & ask.mods_mask == ask.mods_value
    })
}
