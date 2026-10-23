//! Frame timing: whether anything moves at `now`, how often the host
//! should draw while it does, and what waits on time between frames.

use crate::browser::Browser;
use crate::channel::{PAGES, PAGES_PER_BANK};
use crate::wall::{UPDATE_GLOW_MILLISECONDS, UPDATE_MARK_MILLISECONDS};

use super::UPDATE_RING_MILLISECONDS;

/// How long the dot marking a newly received page lasts.
pub(super) const PULSE_MILLISECONDS: u64 = 900;
/// The dot starts this many times a cell's size and shrinks to nothing.
pub(super) const PULSE_SCALE: f64 = 2.2;
pub(super) const PULSE_LIMIT: usize = PAGES;
/// One refresh of a 60 hertz display.
pub(super) const SMOOTH_FRAME_MILLISECONDS: u64 = 16;
/// Once a second, for the age marks of recent updates.
pub(super) const MARK_FRAME_MILLISECONDS: u64 = 1_000;
/// Ten frames a second: each dot shrinks away in nine steps.
pub(super) const PULSE_FRAME_MILLISECONDS: u64 = 100;

impl Browser {
    /// Whether anything is moving at `now`, so the host should keep drawing.
    #[must_use]
    pub fn animating(&self, now: u64) -> bool {
        self.frame_interval(now).is_some()
    }

    /// How often to draw while something moves, in milliseconds: every
    /// display refresh for the page area's transitions, and ten times a
    /// second for the overview's dots, which need no more.
    #[must_use]
    pub fn frame_interval(&self, now: u64) -> Option<u64> {
        // An update glows and is marked with its age where its page is
        // drawn: on the wall and in the list, which show the bank. The
        // overview's ring bursts for every page, while the overview shows.
        let in_bank = |index: usize| index / PAGES_PER_BANK == self.bank;
        let since_update =
            |limit: u64, everywhere: bool| self.updates.iter().any(|&(index, at)| (everywhere || in_bank(index)) && now.saturating_sub(at) < limit);
        let rings = !self.overview_folded && !self.console.open();
        // A held Back or Forward opens its list at its time; Undo ends.
        let deadline = self.hold_deadline(now).into_iter().chain(self.bookmarks_deadline(now)).chain(self.tips_deadline(now)).min();
        if self.wall.animating(now) || self.remote_animating(now) {
            Some(SMOOTH_FRAME_MILLISECONDS)
        } else if deadline.is_some() {
            deadline
        } else if self.pulses.iter().any(|&(_, start)| now.saturating_sub(start) < PULSE_MILLISECONDS)
            || since_update(UPDATE_GLOW_MILLISECONDS, false)
            || (rings && since_update(UPDATE_RING_MILLISECONDS, true))
        {
            Some(PULSE_FRAME_MILLISECONDS)
        } else if since_update(UPDATE_MARK_MILLISECONDS, false) {
            // The "Updated 12s" marks count whole seconds.
            Some(MARK_FRAME_MILLISECONDS)
        } else {
            #[cfg(feature = "interaction")]
            return self.terminal_frame_interval();
            #[cfg(not(feature = "interaction"))]
            None
        }
    }

    /// Advances what waits on time; returns whether anything changed.
    pub fn settle(&mut self, now: u64) -> bool {
        let before = self.pulses.len();
        self.pulses.retain(|&(_, start)| now.saturating_sub(start) < PULSE_MILLISECONDS);
        let updates = self.updates.len();
        self.updates.retain(|&(_, at)| now.saturating_sub(at) < UPDATE_MARK_MILLISECONDS);
        #[cfg(feature = "interaction")]
        let echoes = self.settle_echoes(now);
        #[cfg(not(feature = "interaction"))]
        let echoes = false;
        let held = self.settle_hold(now);
        let tipped = self.tips_settle(now);
        self.pulses.len() != before || self.updates.len() != updates || echoes || held || tipped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::fixture::browser;

    /// An update glows and is marked with its age where its page is drawn,
    /// the wall and the list, which show one bank: a page of another bank
    /// asks for frames only while its ring bursts in the overview. Before
    /// this, any update anywhere drew the whole window ten times a second
    /// for three seconds and once a second for a minute.
    #[test]
    fn an_update_in_another_bank_asks_for_frames_only_while_its_ring_bursts() {
        let (mut browser, _) = browser();
        assert_eq!(browser.bank, 0);
        browser.page_updated(7, 0x401, 100);
        assert_eq!(browser.frame_interval(200), Some(PULSE_FRAME_MILLISECONDS), "the ring in the overview");
        assert_eq!(browser.frame_interval(100 + UPDATE_RING_MILLISECONDS), None, "nothing of it is on screen");
        browser.page_updated(7, 0x101, 100);
        assert_eq!(browser.frame_interval(100 + UPDATE_RING_MILLISECONDS), Some(PULSE_FRAME_MILLISECONDS), "the glow on the wall");
        assert_eq!(browser.frame_interval(100 + UPDATE_GLOW_MILLISECONDS), Some(MARK_FRAME_MILLISECONDS), "the age mark on the wall");
        assert_eq!(browser.frame_interval(100 + UPDATE_MARK_MILLISECONDS), None, "the mark has gone");
    }
}
