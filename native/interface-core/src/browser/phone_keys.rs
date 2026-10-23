//! The paired phone's keys while no page is open for typing (decision 99,
//! finding F-20): they move between pages and nothing else. They go to the
//! page view alone, never to a text field, the strip's question, a panel
//! or a list, and while one of those has the keys they are dropped, so a
//! compromised keyboard process cannot type an address, change a setting,
//! or answer a hand-off. The trusted core passes on only these keys in
//! the first place; this is the interface's half.

use super::{Browser, Key};
use crate::controls::FastextKey;

/// A phone key the page view acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhonePageKey {
    /// A digit of a page number.
    Digit(char),
    /// The left arrow: the page before, or the sub-page before.
    Left,
    /// The right arrow: the page after, or the sub-page after.
    Right,
    /// The up arrow: the page above in the bank.
    Up,
    /// The down arrow: the page below in the bank.
    Down,
    /// A Fastext colour key of the open page.
    Fastext(FastextKey),
}

/// The page view's key a phone names, by its W3C `KeyboardEvent.key`
/// name or as one character: the digits, the four arrows and the four
/// colour keys of the phone's layout (qrkeyboard's `ColorF0Red` to
/// `ColorF3Blue`, Fastext's red, green, yellow and cyan, the engine's
/// REQ-NAV-003), and nothing else.
#[must_use]
pub fn phone_page_key(name: &str) -> Option<PhonePageKey> {
    Some(match name {
        "ArrowLeft" => PhonePageKey::Left,
        "ArrowRight" => PhonePageKey::Right,
        "ArrowUp" => PhonePageKey::Up,
        "ArrowDown" => PhonePageKey::Down,
        "ColorF0Red" => PhonePageKey::Fastext(FastextKey::Red),
        "ColorF1Green" => PhonePageKey::Fastext(FastextKey::Green),
        "ColorF2Yellow" => PhonePageKey::Fastext(FastextKey::Yellow),
        "ColorF3Blue" => PhonePageKey::Fastext(FastextKey::Cyan),
        _ => {
            let mut characters = name.chars();
            match (characters.next(), characters.next()) {
                (Some(digit), None) if digit.is_ascii_digit() => PhonePageKey::Digit(digit),
                _ => return None,
            }
        }
    })
}

impl Browser {
    /// Whether the keys are the strip's now: a text field is being
    /// edited, a panel, popover or list is open, or the strip asks its
    /// question.
    fn keys_are_the_strips(&self) -> bool {
        #[cfg(feature = "links")]
        let asking = self.prompt_shown();
        #[cfg(not(feature = "links"))]
        let asking = false;
        self.editing || self.strip_has_keys() || self.cookies_open() || self.console_open() || asking
    }

    /// A key of the paired phone while no page is open for typing: the
    /// page view's alone, on its press. Returns whether the window needs
    /// drawing again.
    pub fn phone_key(&mut self, down: bool, name: &str, now: u64) -> bool {
        if !down || self.keys_are_the_strips() {
            return false;
        }
        let Some(key) = phone_page_key(name) else { return false };
        match key {
            PhonePageKey::Digit(digit) => self.type_digit(digit),
            PhonePageKey::Fastext(colour) => return self.fastext_key(colour),
            PhonePageKey::Left | PhonePageKey::Right => {
                let right = key == PhonePageKey::Right;
                // On a page with sub-pages the arrows step through them,
                // as the window's own arrows do.
                if let Some(changed) = self.control_key(if right { Key::Right } else { Key::Left }) {
                    return changed;
                }
                self.wall.step(if right { 1 } else { -1 }, now);
            }
            PhonePageKey::Up | PhonePageKey::Down => self.step_page(key == PhonePageKey::Down, now),
        }
        true
    }

    /// Text the paired phone typed while no page is open for typing: its
    /// digits choose a page, as typed on its keys; the rest is dropped.
    pub fn phone_text(&mut self, text: &str, _now: u64) -> bool {
        if self.keys_are_the_strips() {
            return false;
        }
        let mut typed = false;
        for digit in text.chars().filter(char::is_ascii_digit) {
            self.type_digit(digit);
            typed = true;
        }
        typed
    }
}

#[cfg(test)]
mod tests;
