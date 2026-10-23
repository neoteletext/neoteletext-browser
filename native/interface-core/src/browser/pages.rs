//! The channel's pages in the window: the banks, the page chosen by
//! number or in the list, what a page's arrival and update start, and the
//! page the user has opened.

use alloc::string::String;
use alloc::vec::Vec;

use crate::browser::Browser;
use crate::browser::frames::PULSE_LIMIT;
use crate::canvas::PageKey;
use crate::channel::{Channel, PAGES_PER_BANK, PageInfo, Status, page_index, page_number};

use crate::browser::page_list::ROW_HEIGHT;

const DECIMAL_DIGITS: usize = 3;

impl Browser {
    /// A channel's banks and pages. A different channel starts at its
    /// first bank; the same channel again keeps the choice.
    pub fn set_channel(&mut self, channel: Channel) {
        if channel.channel != self.channel.channel {
            self.versions.clear();
            self.bank = 0;
            self.selected = None;
            self.list_scroll = 0.0;
            self.wall.to_top();
            self.pulses.clear();
            self.updates.clear();
        }
        self.channel = channel;
    }

    /// A page that has just arrived, with its title: it appears in the
    /// banks, the list, the page area and the overview, with the
    /// overview's dot.
    pub fn add_page(&mut self, channel: u32, page: u16, title: String, now: u64) {
        if channel != self.channel.channel {
            return;
        }
        let Some(index) = page_index(page) else { return };
        let Some(slot) = self.channel.pages.get_mut(index) else { return };
        let new = !matches!(slot, Some(info) if info.status == Status::Loaded);
        // A folded overview keeps no animation; opened, it draws the page
        // as it is.
        if new && !self.overview_folded && self.pulses.len() < PULSE_LIMIT {
            self.pulses.push((index, now));
        }
        *slot = Some(PageInfo { title, status: Status::Loaded });
        #[cfg(feature = "links")]
        self.show_pending(channel, page, now);
    }

    /// A page has a new version: it glows, and is marked with its age.
    pub fn page_updated(&mut self, channel: u32, page: u16, now: u64) {
        if channel != self.channel.channel {
            return;
        }
        let Some(index) = page_index(page) else { return };
        self.updates.retain(|&(updated, _)| updated != index);
        self.updates.push((index, now));
    }

    pub(super) fn updated_at(&self, index: usize) -> Option<u64> {
        self.updates.iter().find(|&&(updated, _)| updated == index).map(|&(_, at)| at)
    }

    pub(super) fn page_key(&self, index: usize) -> Option<PageKey> {
        page_number(index).map(|page| PageKey { channel: self.channel.channel, page })
    }

    pub(super) fn select_page(&mut self, index: usize, reveal: bool) {
        let bank = index / PAGES_PER_BANK;
        if bank != self.bank {
            self.choose_bank(bank);
        }
        self.selected = Some(index);
        if reveal && let Some(key) = self.page_key(index) {
            self.wall.reveal(key);
            // A zoomed area shows the newly chosen page instead.
            if self.wall.zoomed().is_some() {
                self.wall.switch_zoom(key);
            }
        }
        self.reveal_row(index);
    }

    pub(super) fn choose_bank(&mut self, bank: usize) {
        self.bank = bank;
        self.selected = None;
        self.list_scroll = 0.0;
        self.wall.to_top();
    }

    /// Scrolls the list so that page `index`'s row is in view.
    pub(super) fn reveal_row(&mut self, index: usize) {
        let position = self.bank_pages().iter().position(|&candidate| candidate == index);
        let Some(position) = position else { return };
        #[allow(clippy::cast_precision_loss, reason = "at most 100 rows")]
        let top = position as f64 * ROW_HEIGHT;
        let view = self.regions.list.height();
        if top < self.list_scroll {
            self.list_scroll = top;
        } else if top + ROW_HEIGHT > self.list_scroll + view {
            self.list_scroll = top + ROW_HEIGHT - view;
        }
    }

    /// The indices of the chosen bank's pages.
    pub(super) fn bank_pages(&self) -> Vec<usize> {
        let first = self.bank * PAGES_PER_BANK;
        (first..first + PAGES_PER_BANK).filter(|&index| matches!(self.channel.pages.get(index), Some(Some(_)))).collect()
    }

    /// The page the user has opened, whose sounds play (the engine's
    /// REQ-AUD-012): the zoomed page, or else the page chosen by number or
    /// in the list, as a television goes to the page whose number is typed.
    #[must_use]
    pub fn opened(&self) -> Option<PageKey> {
        self.wall.zoomed().or_else(|| self.selected.and_then(|index| self.page_key(index)))
    }

    /// A digit, from the keyboard or the remote: three choose a page, as on
    /// a television.
    pub(super) fn type_digit(&mut self, digit: char) {
        self.digits.push(digit);
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

    /// The list and the overview follow the zoomed page.
    pub(super) fn follow_zoom(&mut self) {
        if let Some(index) = self.wall.zoomed().and_then(|key| page_index(key.page)) {
            self.selected = Some(index);
            self.reveal_row(index);
        }
    }

    pub(super) fn step_page(&mut self, forward: bool, now: u64) {
        if self.wall.zoomed().is_some() {
            if self.wall.step_zoom(if forward { 1 } else { -1 }, now).is_some() {
                self.follow_zoom();
            }
            return;
        }
        let pages = self.bank_pages();
        let position = self.selected.and_then(|selected| pages.iter().position(|&index| index == selected));
        let next = match (position, forward) {
            (None, _) => pages.first(),
            (Some(position), true) => pages.get(position + 1),
            (Some(position), false) => position.checked_sub(1).and_then(|previous| pages.get(previous)),
        };
        if let Some(&index) = next {
            self.select_page(index, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::fixture::{browser, press_at};
    use crate::browser::frames::PULSE_MILLISECONDS;
    use crate::browser::{Key, Modifiers};

    #[test]
    fn three_digits_choose_a_page_and_a_received_page_pulses() {
        let (mut browser, _) = browser();
        browser.key(Key::Text("4"), Modifiers::default(), 0);
        browser.key(Key::Text("01"), Modifiers::default(), 0);
        assert_eq!(browser.bank, 3);
        assert_eq!(browser.selected, page_index(0x401));
        // Page 402 is not in the test channel: it arrives now.
        browser.add_page(7, 0x402, "New".into(), 100);
        assert!(browser.animating(500));
        assert!(!browser.animating(100 + PULSE_MILLISECONDS));
        assert_eq!(browser.channel.counts().1, 1);
    }

    #[test]
    fn the_opened_page_is_the_zoomed_one_or_else_the_one_chosen() {
        let (mut browser, mut canvas) = browser();
        assert_eq!(browser.opened(), None);
        browser.key(Key::Text("401"), Modifiers::default(), 0);
        assert_eq!(browser.opened(), Some(PageKey { channel: 7, page: 0x401 }));
        let (key, rect) = browser.wall().shown().next().unwrap();
        press_at(&mut browser, &mut canvas, rect);
        assert_eq!(browser.opened(), Some(key), "a zoomed page wins");
    }
}
