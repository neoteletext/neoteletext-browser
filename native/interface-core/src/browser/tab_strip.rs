//! The tab strip (`tab-strip.md`, decisions 69 and 102): tabs of one
//! width in a row that slides, with easing, to show the active one; when
//! they overflow, a count of those not shown, which opens the tab
//! filmstrip (`filmstrip.rs`, REQ-6640); a tab dragged along to a new
//! place, which the trusted core decides; and at its right end the
//! browser's version and release stage (decision 123).

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::browser::toolbar::{TOOLBAR_HEIGHT, TOOLBAR_TOP};
use crate::browser::{Browser, DropPlace, TabInfo};
use crate::geometry::{Point, Rect};

mod drag;
mod draw;

pub(in crate::browser) const STRIP_HEIGHT: f64 = 44.0;
const TAB_TOP: f64 = 6.0;
const TAB_TOP_INACTIVE: f64 = 9.0;
/// Every tab's width, however many there are (decision 102): a tab that
/// shrank as tabs were added moved every other under the pointer.
pub(in crate::browser) const TAB_WIDTH: f64 = 230.0;
const TAB_GAP: f64 = 4.0;
const TAB_TEXT_INSET: f64 = 14.0;
const TAB_CLOSE: f64 = 22.0;
/// A private tab's mark, before its title.
const PRIVATE_MARK: f64 = 10.0;
/// A tab's sound button, before its close button (`tab-strip.md` section 9).
const TAB_SPEAKER: f64 = 24.0;
const NEW_TAB: f64 = 38.0;
const STRIP_MARGIN: f64 = 8.0;
const WINDOW_BUTTON_WIDTH: f64 = 44.0;
const WINDOW_BUTTON_HEIGHT: f64 = 32.0;
const WINDOW_BUTTON_GAP: f64 = 6.0;
/// How far a tab must move with the button held before it is dragged, not
/// chosen: about a finger's tremble.
const DRAG_THRESHOLD: f64 = 4.0;
/// How far below the strip a dragged tab is let go before it leaves the
/// window rather than staying on the strip: a little slack for a hand that
/// drifts while dragging along it.
const TEAR_DISTANCE: f64 = 24.0;
/// The count of tabs that do not fit: "+12".
const COUNT_WIDTH: f64 = 40.0;
/// The line that shows where a dragged tab will go.
const MARKER_WIDTH: f64 = 3.0;
/// The release stage shown after the browser's version (decision 123):
/// the owner's "ensure it says BETA".
const RELEASE_STAGE: &str = "BETA";
/// The room on each side of the version label, which keeps it apart from
/// the new-tab button and the window's own buttons, so that it reads as a
/// label of the strip and not as one of them.
const VERSION_GAP: f64 = 16.0;
/// The least room the tabs keep beside the version label: one tab, the
/// count of the rest and the new-tab button, as `draw_tabs` lays them out
/// when they overflow. With less, the label is left out (decision 123):
/// every tab stays reachable on the strip or through the count.
const LEAST_TABS_ROOM: f64 = TAB_WIDTH + TAB_GAP + COUNT_WIDTH + TAB_GAP * 2.0 + NEW_TAB;

/// A tab being pressed or dragged.
#[derive(Clone, Copy, Debug)]
struct Drag {
    /// The tab's place.
    index: usize,
    /// Where the press was.
    start: Point,
    /// Moved past the threshold: dragged, not chosen.
    moving: bool,
    /// Where it would go if released now.
    target: usize,
}

/// The strip's state in the browser; its slide is the filmstrip's state
/// (REQ-6640 item 11).
#[derive(Debug, Default)]
pub(super) struct TabsUi {
    drag: Option<Drag>,
    /// Where the count was last drawn, and the strip's tabs with the
    /// count, on which the pointer rests to open the filmstrip.
    pub(super) count: Rect,
    pub(super) tabs_area: Rect,
}

/// A coordinate in whole pixels, held within `i32`.
fn whole(value: f64) -> i32 {
    let rounded = libm::round(value).clamp(f64::from(i32::MIN), f64::from(i32::MAX));
    #[expect(clippy::cast_possible_truncation, reason = "rounded and held within i32 above")]
    let whole = rounded as i32;
    whole
}

/// How far the strip's row of tabs is slid to show the active tab whole
/// (REQ-6640 item 8): the least move from `slid` that brings it into a
/// view `view` wide, within the row of `count` tabs, each `step` with its
/// gap.
fn strip_slide(slid: f64, active: usize, count: usize, view: f64, step: f64) -> f64 {
    #[allow(clippy::cast_precision_loss, reason = "a tab's place, below a hundred")]
    let (left, row) = (active as f64 * step, count as f64 * step - TAB_GAP);
    let right = left + TAB_WIDTH;
    let slid = if left < slid {
        left
    } else if right > slid + view {
        right - view
    } else {
        slid
    };
    slid.clamp(0.0, (row - view).max(0.0))
}

/// What lies under a drop (REQ-1700 items 1 to 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropTarget {
    /// The address field, which takes the first file's address.
    Address,
    /// Anywhere else: tabs by this place.
    Tabs(DropPlace),
}

impl Browser {
    /// What lies at `point`, for files dropped there.
    pub fn drop_target(&self, point: Point) -> DropTarget {
        const QUARTERS: f64 = 4.0;
        if self.regions.field_text.contains(point) {
            return DropTarget::Address;
        }
        let tab = self.regions.tabs.iter().find(|tab| tab.tab.contains(point));
        let place = tab.and_then(|tab| {
            let index = u8::try_from(tab.index).ok()?;
            let quarter = tab.tab.width() / QUARTERS;
            Some(if point.x < tab.tab.x0 + quarter {
                DropPlace::Before(index)
            } else if point.x > tab.tab.x1 - quarter {
                DropPlace::After(index)
            } else {
                DropPlace::Replace(index)
            })
        });
        // Over the toolbar they replace the active tab, as a drop on it does.
        let on_toolbar = (TOOLBAR_TOP..TOOLBAR_TOP + TOOLBAR_HEIGHT).contains(&point.y);
        let active = u8::try_from(self.active).ok().filter(|_| on_toolbar).map(DropPlace::Replace);
        DropTarget::Tabs(place.or(active).unwrap_or(DropPlace::NewTabs))
    }
}

impl Browser {
    /// The tabs as the trusted core reports them, and which is active.
    pub fn set_tabs(&mut self, tabs: Vec<TabInfo>, active: usize) {
        let shown = self.tabs.get(self.active).map(|tab| tab.channel);
        self.active = active.min(tabs.len().saturating_sub(1));
        self.tabs = tabs;
        // The field edits the active tab's address, so a change of tab, by
        // any route, ends it; otherwise it goes on showing the other tab's.
        let left = shown.filter(|&shown| self.tabs.get(self.active).map(|tab| tab.channel) != Some(shown));
        if left.is_some() {
            self.stop_editing();
        }
        // The tab left keeps a still of its page (REQ-6640 item 3).
        self.filmstrip_tabs_changed(left);
    }

    /// The channels playing sound, muted or not, as the host's mixers
    /// report. Returns whether that changed what the tabs show.
    pub fn set_sounding(&mut self, sounding: BTreeSet<u32>) -> bool {
        let changed = sounding != self.sounding;
        self.sounding = sounding;
        changed
    }

    /// Whether the tab plays sound, muted or not.
    #[must_use]
    pub fn tab_sounding(&self, tab: &TabInfo) -> bool {
        self.sounding.contains(&tab.channel)
    }

    /// Whether every tab is muted: the mute-all button then unmutes them.
    #[must_use]
    pub fn all_muted(&self) -> bool {
        !self.tabs.is_empty() && self.tabs.iter().all(|tab| tab.muted)
    }

    /// The strip is not on the screen, as in full screen with the strip up:
    /// nothing of it takes the pointer, and with `shut` the filmstrip shuts.
    pub(super) fn tab_strip_forget(&mut self, shut: bool) {
        self.tab_strip.count = Rect::ZERO;
        self.tab_strip.tabs_area = Rect::ZERO;
        if shut {
            self.filmstrip_forget();
        }
    }

    /// Whether a tab is pressed or dragged: the filmstrip then does not
    /// open.
    pub(super) const fn tab_strip_pressed(&self) -> bool {
        self.tab_strip.drag.is_some()
    }

    /// The version label's text (decision 123): the version the trusted
    /// core reports running, which the settings panel and the About panel
    /// show, and the release stage; the stage alone until the trusted core
    /// has reported, which it does as the window opens.
    pub(super) fn version_label(&self) -> String {
        self.running_version().map_or_else(|| String::from(RELEASE_STAGE), |version| format!("{version} {RELEASE_STAGE}"))
    }

    /// The count's tip.
    pub(super) fn tabs_tips(&self, tips: &mut Vec<(Rect, String)>) {
        if self.tab_strip.count != Rect::ZERO {
            tips.push((self.tab_strip.count, String::from("More tabs: rest the pointer on the tabs, or click here, to see them all as pictures")));
        }
    }
}

#[cfg(test)]
mod tests;
