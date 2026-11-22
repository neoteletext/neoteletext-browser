//! The keys a script's manifest claims, as the trusted core turns them
//! into the asks its gate holds input to, and the gate's question of a
//! key event (script-api.md section 4.6; `neoteletext::script_keys` and
//! `interface_core::browser::scripts`). The names are the manifest's,
//! which the channel wrote. Whatever the names, no ask claims a key that
//! is never a script's, every ask leaves the browser's chords to the
//! browser, and a key event an ask names is always one the interface
//! may give a script.
#![no_main]

use interface_core::browser::scripts::{BROWSER_CHORD_MODIFIERS, NEVER_CLAIMABLE_KEYS, may_be_scripts};
use libfuzzer_sys::fuzz_target;
use neoteletext::script_keys::{asks_of, names};
use neoteletext_input::event::KeyCode;
use protocol::decoder::script::ScriptClaims;
use protocol::rules::{KeyKind, MAX_KEY_ASKS, PageSub};

fuzz_target!(|data: &[u8]| {
    let Some((&[mods, kind, a, b, c, d], rest)) = data.split_first_chunk::<6>() else { return };
    let text = String::from_utf8_lossy(rest);
    let claims = ScriptClaims {
        running: true,
        text: mods & 0x80 != 0,
        keys: text.split('\n').map(str::to_owned).collect(),
        ..ScriptClaims::none(PageSub::new(0x100, 0))
    };
    let (asks, _ignored) = asks_of(&claims);
    assert!(asks.keys.len() <= MAX_KEY_ASKS);
    for ask in &asks.keys {
        assert!(ask.mods_mask & BROWSER_CHORD_MODIFIERS == BROWSER_CHORD_MODIFIERS && ask.mods_value == 0, "{ask:?} lets a chord through");
        let key = ask.key.unwrap_or_else(|| panic!("an ask of any key: {ask:?}"));
        if key.kind == KeyKind::Named {
            let name = neoteletext_input::keys::name(key.code);
            assert!(name.is_none_or(|name| !NEVER_CLAIMABLE_KEYS.contains(&name)), "{name:?} is never a script's");
        }
    }
    // The key event of the input's numbers, as a character or a code.
    let number = u32::from_le_bytes([a, b, c, d]);
    let event = if kind & 1 == 0 { char::from_u32(number).map(KeyCode::Character) } else { Some(KeyCode::Functional(number % 0x1_0000)) };
    let Some(event) = event else { return };
    if names(&asks, event, mods) {
        assert!(may_be_scripts(event, mods), "{event:?} with {mods:#04x} is claimed but may not be a script's");
    }
    // Each ask's own key, with each modifier byte.
    for ask in &asks.keys {
        let Some(key) = ask.key else { continue };
        let event = match key.kind {
            KeyKind::Character => char::from_u32(key.code).map(KeyCode::Character),
            KeyKind::Named => Some(KeyCode::Functional(key.code)),
            // A manifest never names a physical key.
            KeyKind::Physical => panic!("an ask of a physical key: {ask:?}"),
        };
        let Some(event) = event else { continue };
        if names(&asks, event, mods) {
            assert!(may_be_scripts(event, mods), "{event:?} with {mods:#04x} is claimed but may not be a script's");
        }
    }
});
