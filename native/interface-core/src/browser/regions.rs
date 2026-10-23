//! Where the parts of the window were last drawn, for hit-testing: the
//! record every drawing file writes and every press reads (requirement
//! 12.3). Its fields are the record, as each panel's `Controls` is.

use alloc::vec::Vec;

use crate::geometry::Rect;

/// Where a tab and its buttons were last drawn.
#[derive(Clone, Copy, Debug)]
pub(super) struct TabRegion {
    /// The tab's place among the tabs.
    pub(super) index: usize,
    pub(super) tab: Rect,
    pub(super) close: Rect,
    /// The sound button, shown while the tab plays sound or is muted.
    pub(super) speaker: Option<Rect>,
}

/// Where the parts of the window were last drawn.
#[derive(Default)]
pub(super) struct Regions {
    pub(super) tabs: Vec<TabRegion>,
    pub(super) mute_all: Rect,
    pub(super) new_tab: Rect,
    pub(super) strip: Rect,
    pub(super) minimise: Rect,
    pub(super) maximise: Rect,
    pub(super) close: Rect,
    pub(super) back: Rect,
    pub(super) forward: Rect,
    pub(super) reload: Rect,
    pub(super) home: Rect,
    pub(super) gear: Rect,
    pub(super) field: Rect,
    pub(super) field_text: Rect,
    pub(super) star: Rect,
    pub(super) menu: Rect,
    pub(super) banks: Vec<Rect>,
    pub(super) list: Rect,
    pub(super) rows: Vec<(Rect, usize)>,
    pub(super) overview: Rect,
    pub(super) overview_toggle: Rect,
    pub(super) cells: Rect,
}

impl Regions {
    /// Forgets the last frame's regions, keeping the lists' room for the
    /// next frame's.
    pub(super) fn clear(&mut self) {
        let mut tabs = core::mem::take(&mut self.tabs);
        let mut banks = core::mem::take(&mut self.banks);
        let mut rows = core::mem::take(&mut self.rows);
        tabs.clear();
        banks.clear();
        rows.clear();
        *self = Self { tabs, banks, rows, ..Self::default() };
    }
}
