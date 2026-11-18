//! What each tab's page costs the viewer's machine (REQ-6626), shown in the
//! developer console's Stats tab as its "Page cost" section: the sound
//! helper's time for the tab's sounds, which the helper reports, and the
//! work its open page's rule table did, which the tab's decoder counts
//! (page-rules.md RULE-72). The decoder's own processor time, read from the
//! operating system, is not yet measured: the trusted core does not start
//! the decoder (the broker does) and is confined from the system's account
//! of other processes, so the figure needs the broker's help (REQ-6626
//! item 1 a); the section says so rather than show a figure it lacks.

use std::collections::{BTreeMap, VecDeque};

use protocol::console::Stat;
use protocol::rules::RuleCounters;

use super::Core;

/// Microseconds in a second: the length of one window of the figures.
const SECOND_US: u64 = 1_000_000;
/// The seconds the average is taken over (REQ-6626 item 1).
const AVERAGE_SECONDS: usize = 10;
/// Parts of a processor core a share is written in: per mille, shown as a
/// percentage with one decimal.
const PER_MILLE: u64 = 1_000;
/// Per mille in one per cent.
const PER_MILLE_PER_CENT: u64 = 10;

/// One tab's figures.
#[derive(Debug, Default)]
pub(super) struct Cost {
    /// The second being counted, and the sound helper's microseconds in it.
    second: u64,
    sound_us: u64,
    /// The sound helper's microseconds in each of the last seconds, newest
    /// last, at most [`AVERAGE_SECONDS`].
    seconds: VecDeque<u64>,
    /// The greatest of them since the tab's page opened.
    greatest_us: u64,
    /// The rule table's counts, as its decoder last told them, and the
    /// counts a second before.
    counters: Option<RuleCounters>,
    before: Option<RuleCounters>,
}

/// Each tab's cost, by its channel.
#[derive(Debug, Default)]
pub(super) struct PageCost {
    tabs: BTreeMap<u32, Cost>,
}

/// A share of one processor core, as the Stats tab writes it.
fn share(microseconds: u64, of_us: u64) -> String {
    let per_mille = microseconds.saturating_mul(PER_MILLE).checked_div(of_us).unwrap_or(0);
    format!("{}.{}%", per_mille / PER_MILLE_PER_CENT, per_mille % PER_MILLE_PER_CENT)
}

impl Cost {
    /// The figures move on to `second`: each second passed closes.
    fn roll(&mut self, second: u64) {
        let held = u64::try_from(AVERAGE_SECONDS).unwrap_or(u64::MAX);
        // A gap of many seconds is as many silent ones, no more than the
        // average holds.
        if second.saturating_sub(self.second) > held {
            self.seconds.push_back(core::mem::take(&mut self.sound_us));
            self.greatest_us = self.greatest_us.max(self.seconds.back().copied().unwrap_or(0));
            self.second = second.saturating_sub(held);
        }
        while self.second < second {
            self.seconds.push_back(core::mem::take(&mut self.sound_us));
            if self.seconds.len() > AVERAGE_SECONDS {
                self.seconds.pop_front();
            }
            self.greatest_us = self.greatest_us.max(self.seconds.back().copied().unwrap_or(0));
            self.second = self.second.saturating_add(1);
        }
    }

    /// The "Page cost" section's figures.
    fn figures(&self) -> Vec<Stat> {
        let stat = |label: &str, value: String| Stat { label: label.into(), value };
        let last = self.seconds.back().copied().unwrap_or(0);
        let total: u64 = self.seconds.iter().sum();
        let window = u64::try_from(self.seconds.len().max(1)).unwrap_or(1).saturating_mul(SECOND_US);
        let mut figures = vec![
            stat("Page cost", String::new()),
            stat("  decoder processor time", String::from("not measured in this build")),
            stat(
                "  sound helper, this tab",
                format!("{} of a core last second, {} over ten, {} at most", share(last, SECOND_US), share(total, window), share(self.greatest_us, SECOND_US)),
            ),
        ];
        let Some(now) = self.counters else {
            figures.push(stat("  rule table", String::from("none")));
            return figures;
        };
        let before = self.before.unwrap_or(RuleCounters { ticks: 0, fired: 0, groups: 0, commands: 0, reports: 0, ..now });
        let both = |now: u32, before: u32| format!("{} a second, {} in all", now.saturating_sub(before), now);
        figures.extend([
            stat("  page", format!("{:03X}/{:04X}", now.page.page, now.page.sub)),
            stat("  ticks run", both(now.ticks, before.ticks)),
            stat("  ticks late", format!("{}, the longest {} µs", now.late_ticks, now.longest_tick_us)),
            stat("  rules fired", both(now.fired, before.fired)),
            stat("  groups run", both(now.groups, before.groups)),
            stat("  commands applied", both(now.commands, before.commands)),
            stat("  commands skipped", now.skipped.to_string()),
            stat("  events dropped", now.events_dropped.to_string()),
            stat("  cues heard, dropped", format!("{}, {}", now.cues, now.cues_dropped)),
            stat("  reports asked for", both(now.reports, before.reports)),
            stat("  variables", now.variables.iter().map(i32::to_string).collect::<Vec<_>>().join(" ")),
        ]);
        figures
    }
}

impl Core {
    /// The sound helper's time for one view's sounds (REQ-6626 item 1 b).
    pub(super) fn sound_spent(&mut self, channel: u32, microseconds: u32) {
        let second = self.now_us().checked_div(SECOND_US).unwrap_or(0);
        let cost = self.page_cost.tabs.entry(channel).or_default();
        if cost.seconds.is_empty() && cost.second == 0 {
            cost.second = second;
        }
        let rolled = cost.second < second;
        cost.roll(second);
        cost.sound_us = cost.sound_us.saturating_add(u64::from(microseconds));
        if rolled {
            let figures = cost.figures();
            self.console.set_page_cost(channel, figures);
        }
    }

    /// The rule table's counts of a view's open page, which its decoder
    /// tells once a second (RULE-72, REQ-6626 item 2).
    pub(super) fn rule_counted(&mut self, channel: u32, counters: RuleCounters) {
        let cost = self.page_cost.tabs.entry(channel).or_default();
        // Another page or sub-code counts from its own opening.
        cost.before = cost.counters.filter(|before| before.page == counters.page);
        cost.counters = Some(counters);
        let figures = cost.figures();
        self.console.set_page_cost(channel, figures);
    }

    /// A view's decoder ended: what it counted goes.
    pub(super) fn page_cost_gone(&mut self, channel: u32) {
        self.page_cost.tabs.remove(&channel);
        self.console.set_page_cost(channel, Vec::new());
    }
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, reason = "a test")]
mod tests {
    use super::*;

    /// REQ-6626 item 1 b: the sound helper's share is that of the last
    /// whole second and the average over the seconds held, at most ten.
    #[test]
    fn the_share_is_of_the_last_second_and_the_average() {
        let mut cost = Cost { second: 100, ..Cost::default() };
        cost.sound_us = 250_000;
        cost.roll(101);
        cost.sound_us = 50_000;
        cost.roll(102);
        assert_eq!(cost.figures()[2].value, "5.0% of a core last second, 15.0% over ten, 25.0% at most");
        cost.roll(200);
        assert_eq!(cost.seconds.len(), AVERAGE_SECONDS, "a long gap is ten silent seconds");
        assert_eq!(cost.figures()[2].value, "0.0% of a core last second, 0.0% over ten, 25.0% at most");
    }
}
