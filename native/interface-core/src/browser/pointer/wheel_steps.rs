//! The wheel beside a zoomed page steps through the open page's sub-pages
//! when it has them, and through the chosen magazine's pages when it has
//! none (REQ-6349; the owner, 16 November 2026: "if a sub page is shown,
//! then the mouse wheel must navigate sub pages not pages").

use crate::browser::Browser;
use crate::theme::WHITE;
use crate::wall::{SLIDE_MILLISECONDS, WHEEL_PER_PAGE};
use crate::wheel_cursor::{Ground, WheelCursor, until_blink};

/// A trackpad keeps at most two steps of movement in hand, as it does for
/// pages (`Wall::wheel_pages`), so a fast flick does not run on.
const CARRY_LIMIT: f64 = 2.0 * WHEEL_PER_PAGE;

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
        if !self.wheel_steps_here() || self.full_screen_over_strip() || self.wall.shown().any(|(_, page)| page.contains(self.pointer)) {
            return None;
        }
        let (up, down) = self.wheel_ways();
        let ground = if self.ground_ink == WHITE { Ground::Dark } else { Ground::Light };
        (up || down).then_some(WheelCursor { up, down, ground })
    }

    /// How long the pointer has had the wheel's shape at `now`: its wheel's
    /// flashes are counted from when it took it.
    fn wheel_shown_for(&self, now: u64) -> u64 {
        now.saturating_sub(self.wheel_since.unwrap_or(now))
    }

    /// Whether the wheel pointer's wheel is lit at `now` (REQ-6356 item 3).
    #[must_use]
    pub fn wheel_lit(&self, now: u64) -> bool {
        crate::wheel_cursor::lit(self.wheel_shown_for(now))
    }

    /// How long until the wheel pointer's wheel next changes, while that
    /// pointer shows.
    pub(in crate::browser) fn wheel_blink_deadline(&self, now: u64) -> Option<u64> {
        self.wheel_cursor().map(|_| until_blink(self.wheel_shown_for(now)))
    }

    /// The wheel pointer came, went, or its wheel changed between lit and
    /// unlit since it was last asked: the window then sets the other
    /// picture. The flashes start when the pointer takes the shape.
    pub(in crate::browser) fn wheel_blink_settle(&mut self, now: u64) -> bool {
        if self.wheel_cursor().is_none() {
            self.wheel_since = None;
            self.wheel_lit = None;
            return false;
        }
        self.wheel_since = self.wheel_since.or(Some(now));
        let lit = Some(self.wheel_lit(now));
        let changed = lit != self.wheel_lit;
        self.wheel_lit = lit;
        changed
    }

    /// Whether the wheel steps through the open page's sub-pages: it has
    /// them, so one of them is what is shown, whichever list was last
    /// clicked. A page without sub-pages leaves the wheel to the pages.
    fn wheel_is_subpages(&self) -> bool {
        self.selected.is_some_and(|index| self.subpage_count(index).is_some())
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
