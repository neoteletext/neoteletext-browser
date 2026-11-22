//! The digits of a page number being typed (REQ-756, REQ-7226): three choose a page,
//! as on a television, and fewer than three are forgotten when no further
//! digit follows in time or the window shows another page or channel
//! (DEC-7225), so that a stray digit never puts a later number out of step.

use super::Browser;
use crate::canvas::PageKey;
use crate::channel::page_index;

/// The digits of a page number a viewer keys: the magazine and two decimal
/// digits (EN 300 706 clause 7.1.2).
const DECIMAL_DIGITS: usize = 3;

/// How long a digit waits for the next before it is forgotten (DEC-7225
/// item 1): long enough to find the next key on a remote control or a
/// phone without hurry, short enough that a digit typed by mistake is gone
/// before the viewer has turned to something else and typed a number.
pub(super) const DIGITS_TIMEOUT_MILLISECONDS: u64 = 3_000;

/// What the window showed when a digit was typed: the channel, the page
/// chosen in the list and the page zoomed to. A change of any of them is
/// another page or another channel, by whatever means it came.
type Place = (u32, Option<usize>, Option<PageKey>);

/// When and where the last pending digit was typed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct DigitsTyped {
    /// When, in the window's milliseconds.
    at: u64,
    /// What the window showed then.
    place: Place,
}

impl Browser {
    fn digits_place(&self) -> Place {
        (self.channel.channel, self.selected, self.wall.zoomed())
    }

    /// Whether digits are pending that no longer count at `now`: the time
    /// for the next has passed, or the window shows another page or channel
    /// than when they were typed (DEC-7225 items 1 and 2).
    fn digits_stale(&self, now: u64) -> bool {
        !self.digits.is_empty() && (now.saturating_sub(self.digits_typed.at) >= DIGITS_TIMEOUT_MILLISECONDS || self.digits_typed.place != self.digits_place())
    }

    /// The digits typed so far that the next digit would join at `now`.
    pub(super) fn pending_digits(&self, now: u64) -> &str {
        if self.digits_stale(now) { "" } else { &self.digits }
    }

    /// How long until the pending digits are forgotten, so that the host
    /// wakes then and the remote's readout stops showing them.
    pub(super) fn digits_deadline(&self, now: u64) -> Option<u64> {
        if self.digits.is_empty() {
            return None;
        }
        // Digits another page made stale are forgotten at the next look.
        let left = if self.digits_stale(now) { 0 } else { self.digits_typed.at.saturating_add(DIGITS_TIMEOUT_MILLISECONDS).saturating_sub(now) };
        Some(left.max(1))
    }

    /// Forgets the pending digits that no longer count; whether it did.
    pub(super) fn digits_settle(&mut self, now: u64) -> bool {
        let stale = self.digits_stale(now);
        if stale {
            self.digits.clear();
        }
        stale
    }

    /// A digit, from the keyboard, the remote or the paired phone: three
    /// choose a page, as on a television.
    pub(super) fn type_digit(&mut self, digit: char, now: u64) {
        self.digits_settle(now);
        self.digits.push(digit);
        self.digits_typed = DigitsTyped { at: now, place: self.digits_place() };
        if self.digits.len() < DECIMAL_DIGITS {
            return;
        }
        let page = u16::from_str_radix(&self.digits, 16).ok();
        self.digits.clear();
        // A number the open page or the channel links elsewhere goes to the
        // trusted core.
        #[cfg(feature = "links")]
        if page.is_some_and(|page| self.link_typed(page)) {
            return;
        }
        if let Some(index) = page.and_then(page_index)
            && matches!(self.channel.pages.get(index), Some(Some(_)))
        {
            self.select_page(index, true);
        }
    }
}

#[cfg(test)]
mod tests;
