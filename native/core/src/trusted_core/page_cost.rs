//! What each tab's page costs the viewer's machine (REQ-6626), shown in the
//! developer console's Stats tab as its "Page cost" section: the sound
//! helper's time for the tab's sounds, which the helper reports, and the
//! work its open page's rule table did, which the tab's decoder counts
//! (page-rules.md RULE-72); and the decoder's own processor time, read
//! once a second from the operating system's account of it by the broker,
//! which started it and is not confined (REQ-6626 item 1 a): never the
//! decoder's own word, since it runs the page. A tab whose decoder has used
//! more than half a core over ten seconds shows a mark the interface draws
//! (REQ-6626).

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
/// The share of one core above which a tab shows its mark, averaged over
/// [`AVERAGE_SECONDS`] (REQ-6626): half a core.
pub(super) const COSTLY_PER_MILLE: u64 = 500;
/// How often each decoder's processor time is read.
pub(super) const SAMPLE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// A decoder's processor time, as the broker read it: the readings of the
/// last ten seconds, and the greatest share of a second.
#[derive(Debug, Default)]
pub(super) struct DecoderTime {
    /// (when, on the trusted core's clock, the decoder's processor time
    /// then), both in microseconds, oldest first, at most one more than
    /// [`AVERAGE_SECONDS`].
    readings: VecDeque<(u64, u64)>,
    /// The greatest share of a core between two readings, per mille.
    greatest: u64,
    /// The last reading failed, and why.
    failed: Option<String>,
}

/// The share of a core, per mille, of `spent` microseconds over `over`.
fn per_mille(spent: u64, over: u64) -> u64 {
    spent.saturating_mul(PER_MILLE).checked_div(over).unwrap_or(0)
}

impl DecoderTime {
    /// A reading at `at_us`: the decoder's processor time since it started.
    pub(super) fn read(&mut self, at_us: u64, total_us: u64) {
        self.failed = None;
        if let Some(&(before_at, before_total)) = self.readings.back() {
            // A decoder that started again counts from nothing.
            if total_us < before_total {
                self.readings.clear();
            } else {
                self.greatest = self.greatest.max(per_mille(total_us.saturating_sub(before_total), at_us.saturating_sub(before_at)));
            }
        }
        self.readings.push_back((at_us, total_us));
        while self.readings.len() > AVERAGE_SECONDS.saturating_add(1) {
            self.readings.pop_front();
        }
    }

    /// The share over the last second and the average over the last ten,
    /// per mille; the average only once ten seconds are held.
    fn shares(&self) -> (Option<u64>, Option<u64>) {
        let share = |from: &(u64, u64), to: &(u64, u64)| per_mille(to.1.saturating_sub(from.1), to.0.saturating_sub(from.0));
        let mut newest = self.readings.iter().rev();
        let last = newest.next();
        let second = last.zip(newest.next()).map(|(to, from)| share(from, to));
        let average = (self.readings.len() > AVERAGE_SECONDS).then(|| self.readings.front().zip(last).map(|(from, to)| share(from, to))).flatten();
        (second, average)
    }

    /// Whether the decoder has used more than half a core over the last
    /// ten seconds (REQ-6626): the tab's mark.
    pub(super) fn costly(&self) -> bool {
        self.shares().1.is_some_and(|average| average > COSTLY_PER_MILLE)
    }

    /// The Stats tab's line.
    fn figure(&self) -> String {
        if let Some(why) = &self.failed {
            return format!("not read: {why}");
        }
        let written = |per_mille: u64| format!("{}.{}%", per_mille / PER_MILLE_PER_CENT, per_mille % PER_MILLE_PER_CENT);
        match self.shares() {
            (None, _) => String::from("being read"),
            (Some(second), average) => format!(
                "{} of a core last second, {}, {} at most",
                written(second),
                average.map_or_else(|| String::from("not yet ten seconds"), |average| format!("{} over ten", written(average))),
                written(self.greatest)
            ),
        }
    }
}

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
    /// The decoder's own processor time (REQ-6626 item 1 a).
    pub(super) decoder: DecoderTime,
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
            stat("  decoder processor time", self.decoder.figure()),
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
            stat("  frames late", now.late_frames.to_string()),
            stat("  rules fired", both(now.fired, before.fired)),
            stat("  groups run", both(now.groups, before.groups)),
            stat("  commands applied", both(now.commands, before.commands)),
            stat("  commands skipped", now.skipped.to_string()),
            stat("  events dropped", now.events_dropped.to_string()),
            stat("  events handled, waited", format!("{}, {}", now.events_handled, now.events_waited)),
            stat("  sequence starts late, skipped", format!("{}, {}", now.starts_late, now.starts_skipped)),
            stat("  cues heard", now.cues.to_string()),
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
    #[cfg(feature = "interaction")]
    pub(super) fn rule_counted(&mut self, channel: u32, counters: RuleCounters) {
        let cost = self.page_cost.tabs.entry(channel).or_default();
        // Another page or sub-code counts from its own opening.
        cost.before = cost.counters.filter(|before| before.page == counters.page);
        cost.counters = Some(counters);
        let figures = cost.figures();
        self.console.set_page_cost(channel, figures);
    }

    /// Reads every decoder's processor time from the broker (REQ-6626
    /// item 1 a), and tells the interface again when a tab's mark comes or
    /// goes.
    pub(super) fn read_decoder_times(&mut self) -> Result<(), String> {
        let at_us = u64::try_from(self.started.elapsed().as_micros()).unwrap_or(u64::MAX);
        let before: Vec<u32> = self.costly_channels();
        let read: Vec<(u32, std::io::Result<u64>)> = self.decoders.iter().map(|(channel, decoder)| (*channel, decoder.processor_time_us())).collect();
        for (channel, spent) in read {
            let cost = self.page_cost.tabs.entry(channel).or_default();
            match spent {
                Ok(total_us) => cost.decoder.read(at_us, total_us),
                Err(error) => cost.decoder.failed = Some(error.to_string()),
            }
            let figures = cost.figures();
            self.console.set_page_cost(channel, figures);
        }
        if self.costly_channels() != before {
            self.send_tabs()?;
        }
        Ok(())
    }

    /// The channels whose tab shows the mark of a page that costs the
    /// machine (REQ-6626).
    pub(super) fn costly_channels(&self) -> Vec<u32> {
        self.page_cost.tabs.iter().filter(|(_, cost)| cost.decoder.costly()).map(|(channel, _)| *channel).collect()
    }

    /// The tabs as the interface draws them, each marked when its page
    /// costs the machine (REQ-6626).
    pub(super) fn tab_infos(&self) -> Vec<interface_core::browser::TabInfo> {
        let costly = self.costly_channels();
        let mut tabs = self.session.tab_infos();
        for tab in &mut tabs {
            tab.costly = costly.contains(&tab.channel);
        }
        tabs
    }

    /// A view's decoder ended: what it counted goes.
    #[cfg(feature = "interaction")]
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

    /// REQ-6626 item 1 a: the decoder's processor time is the operating
    /// system's account, read through the broker's request for the child
    /// it started: a decoder made to spin, which says nothing at all, shows
    /// most of a core; one that sleeps shows next to none.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_spinning_decoder_shows_its_true_processor_time() {
        use super::super::test_support::{Sink, stand_in, test_core};
        let mut core = test_core();
        let (sink, _written) = std::sync::mpsc::channel();
        let spinning = stand_in("/bin/sh", &["-c", "while :; do :; done"], confine::Role::Decoder);
        core.decoders.insert(1, super::super::processes::Decoder::new(spinning, neoteletext::outbox::Outbox::new(Sink(sink.clone())), 1));
        let sleeping = stand_in("/bin/sleep", &["60"], confine::Role::Decoder);
        core.decoders.insert(2, super::super::processes::Decoder::new(sleeping, neoteletext::outbox::Outbox::new(Sink(sink)), 1));
        core.read_decoder_times().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1_200));
        core.read_decoder_times().unwrap();
        let second = |channel: u32| core.page_cost.tabs[&channel].decoder.shares().0.unwrap();
        assert!(second(1) > COSTLY_PER_MILLE, "the spinning decoder: {} a thousand", second(1));
        assert!(second(2) < PER_MILLE / 20, "the sleeping decoder: {} a thousand", second(2));
        for decoder in std::mem::take(&mut core.decoders).into_values() {
            super::super::processes::stop_decoder(decoder);
        }
    }

    /// REQ-6626 item 1 a and the tab's mark: a decoder's share over a second
    /// and over ten from the broker's readings; the mark after ten seconds
    /// above half a core, gone when the average falls below.
    #[test]
    fn a_decoders_share_and_its_mark() {
        let mut time = DecoderTime::default();
        let second = 1_000_000_u64;
        time.read(0, 0);
        assert_eq!(time.figure(), "being read");
        // Three-quarters of a core for nine seconds: no mark yet.
        for at in 1..=9 {
            time.read(at * second, at * 750_000);
            assert!(!time.costly(), "not ten seconds yet");
        }
        assert_eq!(time.figure(), "75.0% of a core last second, not yet ten seconds, 75.0% at most");
        time.read(10 * second, 10 * 750_000);
        assert!(time.costly(), "ten seconds above half a core");
        // Idle: the average falls below half after a few seconds.
        let mut total = 10 * 750_000;
        let mut at = 10;
        while time.costly() {
            at += 1;
            total += 10_000;
            time.read(at * second, total);
            assert!(at < 20, "the mark goes");
        }
        assert_eq!(at, 14, "75 × 6 + 1 × 4 is under 500 a thousand over ten: {}", time.figure());
        // A decoder started again counts from nothing.
        time.read(30 * second, 5);
        assert_eq!(time.figure(), "being read");
    }
}
