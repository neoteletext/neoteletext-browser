//! The tab strip (`tab-strip.md`, decisions 69 and 102): tabs of one
//! width, the active one always shown; when they overflow, a count, and a
//! panel joined under the strip into which the rest wrap once the pointer
//! rests on the tabs; and a tab dragged along to a new place, which the
//! trusted core decides.

use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;

use crate::browser::{Browser, TabInfo};
use crate::geometry::{Point, Rect};

mod drag;
mod draw;

pub(in crate::browser) const STRIP_HEIGHT: f64 = 44.0;
const TAB_TOP: f64 = 6.0;
const TAB_TOP_INACTIVE: f64 = 9.0;
/// Every tab's width, however many there are (decision 102): a tab that
/// shrank as tabs were added moved every other under the pointer.
const TAB_WIDTH: f64 = 230.0;
const TAB_GAP: f64 = 4.0;
const TAB_TEXT_INSET: f64 = 14.0;
const TAB_CLOSE: f64 = 22.0;
/// A private tab's mark, before its title.
const PRIVATE_MARK: f64 = 10.0;
/// A tab's sound button, before its close button, as Chrome places it.
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
/// How long the pointer rests on the tabs before the overflow panel opens
/// (decision 102): a hand crossing the strip to the toolbar takes about a
/// tenth of a second, and opening at once laid the panel over the toolbar
/// in its way.
const PANEL_DELAY_MILLISECONDS: u64 = 300;
/// A row of the overflow panel: a tab's height and the gap below it.
const PANEL_ROW: f64 = STRIP_HEIGHT - TAB_TOP_INACTIVE + TAB_GAP;
/// How dark the window becomes below the open overflow panel, as behind a
/// dialog (decision 102): less than a zoomed page's wall, since the panel
/// opens and shuts as the pointer passes.
const PANEL_DIM: f32 = 0.55;
/// The panel's edge at its foot, in the selection's colour, and the
/// shadow below it: each band of the shadow and how much it darkens,
/// fading out.
const PANEL_EDGE: f64 = 2.0;
const SHADOW_BAND: f64 = 3.0;
const SHADOW: [f32; 4] = [0.5, 0.35, 0.2, 0.1];
/// The line that shows where a dragged tab will go.
const MARKER_WIDTH: f64 = 3.0;

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

/// The strip's state in the browser.
#[derive(Debug, Default)]
pub(super) struct TabsUi {
    /// The overflow panel shows.
    open: bool,
    /// Since when the pointer has rested on the tabs, the panel shut.
    resting: Option<u64>,
    /// The count was pressed to shut the panel: it stays shut until the
    /// pointer leaves the strip.
    dismissed: bool,
    drag: Option<Drag>,
    /// Where the panel and the count were last drawn, and the strip's tabs
    /// with the count, on which the pointer rests to open the panel.
    panel: Rect,
    count: Rect,
    tabs_area: Rect,
    /// The tabs the strip showed while the panel was open, and how many
    /// tabs there were: they stay in place until it shuts, so a tab chosen
    /// in the panel does not jump to the strip under the pointer.
    held: Option<(Vec<usize>, usize)>,
    /// Some tabs did not fit on the strip.
    overflowing: bool,
    /// Those tabs, the strip's tab width, and its left and right ends, for
    /// the panel drawn at the end of the frame.
    rest: (Vec<usize>, f64, f64, f64),
}

/// A coordinate in whole pixels, held within `i32`.
fn whole(value: f64) -> i32 {
    let rounded = libm::round(value).clamp(f64::from(i32::MIN), f64::from(i32::MAX));
    #[expect(clippy::cast_possible_truncation, reason = "rounded and held within i32 above")]
    let whole = rounded as i32;
    whole
}

/// Which tabs the strip shows, in order, and which wrap into the panel: the
/// first that fit, the active one taking the last place when it would not.
fn arrangement(count: usize, active: usize, fit: usize) -> (Vec<usize>, Vec<usize>) {
    if count <= fit {
        return ((0..count).collect(), Vec::new());
    }
    let mut shown: Vec<usize> = (0..fit).collect();
    if active >= fit
        && let Some(last) = shown.last_mut()
    {
        *last = active;
    }
    let rest = rest_of(count, &shown);
    (shown, rest)
}

/// The tabs of `count` that `shown` leaves for the panel, in order.
fn rest_of(count: usize, shown: &[usize]) -> Vec<usize> {
    (0..count).filter(|index| !shown.contains(index)).collect()
}

impl Browser {
    /// The tabs as the trusted core reports them, and which is active.
    pub fn set_tabs(&mut self, tabs: Vec<TabInfo>, active: usize) {
        self.active = active.min(tabs.len().saturating_sub(1));
        self.tabs = tabs;
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
    /// nothing of it takes the pointer, and with `shut` the panel shuts.
    pub(super) fn tab_strip_forget(&mut self, shut: bool) {
        self.tab_strip.count = Rect::ZERO;
        self.tab_strip.panel = Rect::ZERO;
        self.tab_strip.tabs_area = Rect::ZERO;
        if shut {
            self.tab_strip.open = false;
            self.tab_strip.resting = None;
        }
    }

    /// The count's tip.
    pub(super) fn tabs_tips(&self, tips: &mut Vec<(Rect, String)>) {
        if self.tab_strip.count != Rect::ZERO {
            tips.push((self.tab_strip.count, String::from("More tabs: rest the pointer on the tabs, or click here, to see them all")));
        }
    }
}

#[cfg(test)]
mod tests;
