//! What each tab's page costs the viewer's machine (REQ-6626): the
//! processor time of the tab's decoder, read by this core from the
//! operating system, and the sound helper's time for the tab's sounds,
//! which the helper reports. The decoder's own work counts arrive with the
//! rule table's counters (RULE-72).

use std::collections::BTreeMap;

use super::Core;

/// What is known of one tab's cost.
#[derive(Debug, Default)]
pub(super) struct Cost {
    /// Microseconds the sound helper spent rendering the tab's sounds,
    /// since the tab's decoder started.
    pub(super) sound_us: u64,
}

/// Each tab's cost, by its channel.
#[derive(Debug, Default)]
pub(super) struct PageCost {
    pub(super) tabs: BTreeMap<u32, Cost>,
}

impl Core {
    /// The sound helper's time for one view's sounds (REQ-6626 item 1 b).
    pub(super) fn sound_spent(&mut self, channel: u32, microseconds: u32) {
        let cost = self.page_cost.tabs.entry(channel).or_default();
        cost.sound_us = cost.sound_us.saturating_add(u64::from(microseconds));
    }
}
