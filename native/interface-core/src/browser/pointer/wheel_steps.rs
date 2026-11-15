//! The wheel beside a zoomed page steps through the list the viewer last
//! chose from: the chosen magazine's pages, or the chosen page's sub-pages
//! (REQ-6349).

use crate::browser::Browser;
use crate::wall::{SLIDE_MILLISECONDS, WHEEL_PER_PAGE};

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
