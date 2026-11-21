//! Which keys the open page asks for (REQ-7037): the keys its rule table
//! names, as the trusted core passed them on from the page's decoder. A
//! key one of them names is the page's; every other key stays the
//! browser's own viewer's, so that a page that uses some keys leaves page
//! numbers, the Fastext keys, Index, reveal, hold and mix working
//! (REQ-756, REQ-890).

use alloc::vec::Vec;

use neoteletext_input::event::{KeyCode, printable};

/// How a table names a key (neoteletext-common's `docs/page-rules.md`
/// RULE-10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyNamed {
    /// Any key.
    Any,
    /// The character the active layout gives the key without Shift.
    Character(char),
    /// A functional key, by its code in the input protocol's table.
    Named(u32),
    /// The key that carries the character on a United States layout.
    Physical(char),
}

/// A key a table asks for, with the modifiers it compares (RULE-9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AskedKey {
    /// The key.
    pub key: KeyNamed,
    /// The modifier bits compared, as DOC-1935 numbers them.
    pub mods_mask: u8,
    /// What they must be.
    pub mods_value: u8,
}

impl AskedKey {
    /// Any key, whatever the modifiers: the whole keyboard.
    pub const EVERY_KEY: Self = Self { key: KeyNamed::Any, mods_mask: 0, mods_value: 0 };

    /// Whether this names the key of an event, by RULE-9 and RULE-10 as
    /// the page's evaluator matches a `key` trigger: the key, and the
    /// event's modifiers under the mask equal to the value. A physical
    /// key is the event's `base` when it has one, else its `key` when
    /// that is a character.
    fn names(&self, key: KeyCode, base: Option<char>, mods: u8) -> bool {
        let named = match self.key {
            KeyNamed::Any => true,
            KeyNamed::Character(character) => key == KeyCode::Character(character),
            KeyNamed::Named(code) => key == KeyCode::Functional(code),
            KeyNamed::Physical(character) => base.map_or(key == KeyCode::Character(character), |base| base == character),
        };
        named && mods & self.mods_mask == self.mods_value
    }
}

/// What of the keyboard the open page's table asks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AskedKeys {
    /// Text that is no key: a paste, a composition and its committed
    /// text.
    pub text: bool,
    /// The keys it names.
    pub keys: Vec<AskedKey>,
}

impl AskedKeys {
    /// Every key and text: what a table that reports everything asks for.
    #[must_use]
    pub fn everything() -> Self {
        Self { text: true, keys: alloc::vec![AskedKey::EVERY_KEY] }
    }

    /// Whether the page asks for a key: one its table names, or, while
    /// it asks for text, any key that types, since a page being typed
    /// into must not have a letter or a digit act on the browser instead.
    /// A key types when it is a character key or `text`, what it typed,
    /// has a printable character.
    pub(super) fn takes(&self, key: KeyCode, mods: u8, text: Option<&str>, base: Option<char>) -> bool {
        let types = matches!(key, KeyCode::Character(_)) || text.is_some_and(|text| text.chars().any(printable));
        (self.text && types) || self.keys.iter().any(|asked| asked.names(key, base, mods))
    }
}
