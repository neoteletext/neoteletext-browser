//! A live channel's invalid packets in the console (neoteletext-common's
//! REQ-1462, requirement 19.8 version 2): the engine discards and counts
//! them; the trusted core writes one warning line to the Log at the end
//! of the 20-second window that began when the count rose, never one a
//! packet, and shows the count in Stats.

use std::time::{Duration, Instant};

use neoteletext_web::consts::INVALID_PACKET_REPORT_SECONDS;
use protocol::console::Level;

use super::Core;

/// The window a report gathers (`INVALID_PACKET_REPORT_SECONDS`).
const WINDOW: Duration = Duration::from_secs(INVALID_PACKET_REPORT_SECONDS);

/// One channel's count, as the engine last gave it, and its report.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct InvalidReport {
    total: u64,
    /// The total the last line gave.
    reported: u64,
    /// The end of the window the count rose in, while one is open.
    window_end: Option<Instant>,
}

impl Core {
    /// The engine's count of invalid packets on `channel`'s connection
    /// since it opened: a rise opens a window when none is open.
    pub(super) fn invalid_packets(&mut self, channel: u32, total: u64) {
        let report = self.invalid.entry(channel).or_default();
        // A new connection counts from 0 again.
        if total < report.total {
            report.reported = 0;
        }
        if total > report.total && report.window_end.is_none() {
            let now = Instant::now();
            report.window_end = Some(now.checked_add(WINDOW).unwrap_or(now));
        }
        report.total = total;
        self.console.invalid(channel, total);
    }

    /// When the next window ends, if one is open.
    pub(super) fn invalid_due_at(&self) -> Option<Instant> {
        self.invalid.values().filter_map(|report| report.window_end).min()
    }

    /// Writes one line for each window that has ended: the total and the
    /// window's count.
    pub(super) fn invalid_tick(&mut self, now: Instant) {
        let mut lines = Vec::new();
        for (&channel, report) in &mut self.invalid {
            if report.window_end.is_some_and(|end| now >= end) {
                report.window_end = None;
                let rose = report.total.saturating_sub(report.reported);
                report.reported = report.total;
                if rose > 0 {
                    lines.push((
                        channel,
                        format!(
                            "channel: {rose} invalid packets discarded in the last {INVALID_PACKET_REPORT_SECONDS} seconds, {} since the connection opened",
                            report.total
                        ),
                    ));
                }
            }
        }
        let at = self.now();
        for (channel, line) in lines {
            self.console.log(Some(channel), Level::Warning, at, &line);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]
mod tests {
    use super::super::test_support::Test;
    use super::*;

    /// The tab's channel on a console that shows it.
    fn watched(test: &mut Test) -> u32 {
        test.core.session.open("news.invalid").unwrap();
        let channel = test.core.session.tabs[0].channel.id;
        test.core.console.set_open(true);
        test.core.console.log(Some(channel), Level::Info, 0, "the first line, already shown");
        test.core.console.updates(Some(channel));
        channel
    }

    /// The lines of invalid packets written since last asked.
    fn lines(test: &mut Test, channel: u32) -> Vec<String> {
        let updates = test.core.console.updates(Some(channel));
        updates.iter().flat_map(|update| update.entries.iter()).filter(|entry| entry.text.contains("invalid packets")).map(|entry| entry.text.clone()).collect()
    }

    // REQ-1462: the first line comes when the window ends, with the
    // window's count and the total, and never a line a packet.
    #[test]
    fn one_line_ends_each_window_with_its_count_and_the_total() {
        let mut test = Test::new("invalid");
        let channel = watched(&mut test);
        test.core.invalid_packets(channel, 3);
        let end = test.core.invalid_due_at().unwrap();
        test.core.invalid_tick(end.checked_sub(Duration::from_secs(1)).unwrap());
        assert_eq!(lines(&mut test, channel), [] as [String; 0], "the window is still open");
        test.core.invalid_packets(channel, 1_000);
        test.core.invalid_tick(end);
        let written = lines(&mut test, channel);
        assert_eq!(written.len(), 1, "one line for a flood");
        assert!(written[0].contains("1000 invalid packets discarded in the last 20 seconds, 1000 since"), "{}", written[0]);
        assert_eq!(test.core.invalid_due_at(), None, "a quiet time opens no window");
    }

    // REQ-1462: the next rise opens the next window, counting only itself;
    // a quiet window writes nothing; a new connection counts from 0.
    #[test]
    fn the_next_window_counts_only_its_own_and_a_new_connection_begins_again() {
        let mut test = Test::new("invalid windows");
        let channel = watched(&mut test);
        test.core.invalid_packets(channel, 5);
        test.core.invalid_tick(test.core.invalid_due_at().unwrap());
        assert_eq!(lines(&mut test, channel).len(), 1);
        test.core.invalid_packets(channel, 5);
        assert_eq!(test.core.invalid_due_at(), None, "no rise, no window");
        test.core.invalid_packets(channel, 7);
        test.core.invalid_tick(test.core.invalid_due_at().unwrap());
        let written = lines(&mut test, channel);
        assert!(written[0].contains("2 invalid packets discarded") && written[0].contains("7 since"), "{}", written[0]);
        test.core.invalid_packets(channel, 1);
        test.core.invalid_packets(channel, 4);
        test.core.invalid_tick(test.core.invalid_due_at().unwrap());
        let written = lines(&mut test, channel);
        assert!(written[0].contains("4 invalid packets discarded"), "{}", written[0]);
    }

    // REQ-1462, requirement 19.8: Stats shows the count.
    #[test]
    fn stats_show_the_count() {
        let mut test = Test::new("invalid stats");
        let channel = watched(&mut test);
        test.core.invalid_packets(channel, 12);
        let updates = test.core.console.updates(Some(channel));
        let stats = updates.iter().find_map(|update| update.stats.clone()).unwrap();
        assert!(stats.iter().any(|stat| stat.label.contains("invalid") && stat.value == "12"), "{stats:?}");
    }
}
