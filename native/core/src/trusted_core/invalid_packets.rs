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
