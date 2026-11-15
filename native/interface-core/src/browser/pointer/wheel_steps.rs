//! The wheel beside a zoomed page steps through the list the viewer last
//! chose from: the chosen magazine's pages, or the chosen page's sub-pages
//! (REQ-6349).

use crate::browser::Browser;
use crate::theme::WHITE;
use crate::wall::{SLIDE_MILLISECONDS, WHEEL_PER_PAGE};
use crate::wheel_cursor::{Ground, WheelCursor, until_blink};

/// A trackpad keeps at most two steps of movement in hand, as it does for
/// pages (`Wall::wheel_pages`), so a fast flick does not run on.
const CARRY_LIMIT: f64 = 2.0 * WHEEL_PER_PAGE;

/// The list the wheel steps through beside a zoomed page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::browser) enum WheelList {
    /// The pages: the last click on a list was on the magazines or the
    /// page list.
    Pages,
    /// The chosen page's sub-pages: the last click on a list was on the
    /// sub-page list.
    SubPages,
}

impl Browser {
    /// Whether a notch of the wheel under the pointer steps a page or a
    /// sub-page: beside a zoomed page, in the page area, with nothing over
    /// it and off the page-size bar.
    pub(super) fn wheel_steps_here(&self) -> bool {
        !self.over_overlay() && self.wall.zoomed().is_some() && self.wall.contains(self.pointer) && !self.wall.over_bar(self.pointer)
    }

    /// The ways there are items to step to, before and after the one
    /// shown, in the list the wheel steps through (REQ-6356 item 1).
    fn wheel_ways(&self) -> (bool, bool) {
        if !self.wheel_is_subpages() {
            return self.wall.zoom_ways();
        }
        let count = self.selected.and_then(|index| self.subpage_count(index)).unwrap_or(0);
        // The place shown is the open page's only when the controls are
        // this page's of this channel (DEC-6358 item 3).
        let page = self.selected.and_then(crate::channel::page_number);
        let place = page.and_then(|page| self.chosen_place(page)).unwrap_or(0);
        (place > 0, place.saturating_add(1) < count)
    }

    /// The wheel pointer, where the wheel steps and there is somewhere to
    /// step to: beside the open page and never over it, where the pointer
    /// points at the page's own cells and links and the wheel may be the
    /// channel's (REQ-6356 items 1, 4 and 5).
    pub(super) fn wheel_cursor(&self) -> Option<WheelCursor> {
        if !self.wheel_steps_here() || self.wall.shown().any(|(_, page)| page.contains(self.pointer)) {
            return None;
        }
        let (up, down) = self.wheel_ways();
        let ground = if self.ground_ink == WHITE { Ground::Dark } else { Ground::Light };
        (up || down).then_some(WheelCursor { up, down, ground })
    }

    /// How long until the wheel pointer's wheel next blinks, while that
    /// pointer shows.
    pub(in crate::browser) fn wheel_blink_deadline(&self, now: u64) -> Option<u64> {
        self.wheel_cursor().map(|_| until_blink(now))
    }

    /// The wheel pointer's wheel changed between lit and unlit since it
    /// was last asked: the window then sets the other picture.
    pub(in crate::browser) fn wheel_blink_settle(&mut self, now: u64) -> bool {
        let lit = self.wheel_cursor().map(|_| crate::wheel_cursor::lit(now));
        let changed = lit.is_some() && lit != self.wheel_lit;
        self.wheel_lit = lit;
        changed
    }

    /// Whether the wheel steps through the chosen page's sub-pages: the
    /// sub-page list was the list last chosen from, and the chosen page
    /// has one. A page without sub-pages leaves the wheel to the pages.
    fn wheel_is_subpages(&self) -> bool {
        self.wheel_list == WheelList::SubPages && self.selected.is_some_and(|index| self.subpage_count(index).is_some())
    }

    /// A mouse wheel's notch beside a zoomed page: one sub-page for any
    /// notch, whatever size the platform reports for it, and at most one
    /// for each `SLIDE_MILLISECONDS`, the pace `Wall::wheel_notch` keeps
    /// for pages, so that a notch the platform reports twice is one step;
    /// or one page as `Wall::wheel_notch` steps them.
    pub(super) fn wheel_notch(&mut self, lines: f64, now: u64) {
        if !self.wheel_is_subpages() {
            if self.wall.wheel_notch(lines, now).is_some() {
                self.follow_zoom();
            }
        } else if lines != 0.0 && self.wheel_step_at.is_none_or(|at| now.saturating_sub(at) >= SLIDE_MILLISECONDS) {
            self.wheel_step_at = Some(now);
            self.step_subpages(lines > 0.0, now);
        }
    }

    /// A trackpad's movement beside a zoomed page: one sub-page for every
    /// `WHEEL_PER_PAGE` pixels, or the pages as `Wall::wheel_pages` steps
    /// them.
    pub(super) fn wheel_pages(&mut self, distance: f64, now: u64) {
        if !self.wheel_is_subpages() {
            self.wheel_step_carry = 0.0;
            if self.wall.wheel_pages(distance, now).is_some() {
                self.follow_zoom();
            }
            return;
        }
        self.wheel_step_carry = (self.wheel_step_carry + distance).clamp(-CARRY_LIMIT, CARRY_LIMIT);
        while self.wheel_step_carry.abs() >= WHEEL_PER_PAGE {
            let forward = self.wheel_step_carry > 0.0;
            self.wheel_step_carry -= if forward { WHEEL_PER_PAGE } else { -WHEEL_PER_PAGE };
            self.step_subpages(forward, now);
        }
    }
}
