//! The timed test's display (neoteletext-common's `docs/page-rules.md`
//! RULE-66), only in the build the test makes (the feature `timed-test`).
//!
//! The test runs the browser on a private X display, which composes
//! nothing and has no refresh: the interface presents each frame the
//! moment it draws it, and the platform gives it no frame times, so a
//! shipped interface there sends no `FramePhase` (RULE-98 item 1). For the
//! lock's figures the test needs a display that composes at a refresh
//! rate: this one composes at 60 hertz on the interface's own clock. A
//! frame asked for is drawn and presented at the next composition, as a
//! compositor would show it, and the interface tells its phase as RULE-98
//! says a platform's frame times are told.

use std::time::{Duration, Instant};

use protocol::rules::FramePhase;

/// The test display's refresh period: 60 hertz, rounded to the
/// microsecond.
const PERIOD_MICROSECONDS: u32 = 16_667;
/// How often the phase is told again: twice within the time a decoder
/// keeps it (RULE-98 item 4's `PHASE_STALE_MILLISECONDS`), so it is never
/// stale while the window is open.
const TELL_EVERY: Duration = Duration::from_millis(protocol::rules::PHASE_STALE_MILLISECONDS / 2);

/// The display's compositions and what waits for the next.
#[derive(Debug)]
pub(super) struct TestDisplay {
    epoch: Instant,
    /// A frame was asked for and waits for the next composition.
    pub(super) owed: bool,
    /// The composition at hand is being drawn: a frame asked for now is
    /// drawn in it.
    pub(super) composing: bool,
    /// When the phase was last told.
    told: Option<Instant>,
}

impl TestDisplay {
    pub(super) fn new() -> Self {
        Self { epoch: Instant::now(), owed: false, composing: false, told: None }
    }

    fn period() -> Duration {
        Duration::from_micros(u64::from(PERIOD_MICROSECONDS))
    }

    /// The first composition at or after `now`.
    pub(super) fn next_composition(&self, now: Instant) -> Instant {
        let since = u64::try_from(now.saturating_duration_since(self.epoch).as_micros()).unwrap_or(u64::MAX);
        let period = u64::from(PERIOD_MICROSECONDS);
        let compositions = since.div_ceil(period);
        self.epoch.checked_add(Duration::from_micros(compositions.saturating_mul(period))).unwrap_or(now)
    }

    /// Whether `now` is a composition's moment: within a millisecond after
    /// one, the loop's own lateness in waking.
    pub(super) fn composes_at(&self, now: Instant) -> bool {
        let previous = self.next_composition(now.checked_sub(Self::period()).unwrap_or(now));
        let lateness = Duration::from_millis(1);
        now.saturating_duration_since(previous) <= lateness || self.next_composition(now) == now
    }

    /// The phase to tell now, when it is due again.
    pub(super) fn phase_due(&mut self, now: Instant) -> Option<FramePhase> {
        if self.told.is_some_and(|told| now.saturating_duration_since(told) < TELL_EVERY) {
            return None;
        }
        self.told = Some(now);
        let next_in = self.next_composition(now).saturating_duration_since(now);
        FramePhase::at(PERIOD_MICROSECONDS, u64::try_from(next_in.as_micros()).unwrap_or(0), 0)
    }

    /// When the loop must wake for the display: the next composition when
    /// a frame is owed, and the next telling of the phase.
    pub(super) fn wake(&self, now: Instant) -> Instant {
        let tell = self.told.and_then(|told| told.checked_add(TELL_EVERY)).unwrap_or(now);
        if self.owed { tell.min(self.next_composition(now)) } else { tell }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::arithmetic_side_effects, reason = "a test")]
mod tests {
    use super::*;

    /// RULE-66: compositions every 16,667 microseconds, and the phase told
    /// is the time to the next.
    #[test]
    fn the_test_display_composes_at_sixty_hertz() {
        let mut display = TestDisplay::new();
        let at = display.epoch + Duration::from_micros(5_000);
        assert_eq!(display.next_composition(at), display.epoch + TestDisplay::period());
        assert!(display.composes_at(display.epoch + TestDisplay::period() + Duration::from_micros(300)));
        assert!(!display.composes_at(at));
        let phase = display.phase_due(at).unwrap();
        assert_eq!(phase.period_us, PERIOD_MICROSECONDS);
        assert_eq!(phase.next_in_us, PERIOD_MICROSECONDS - 5_000);
        assert_eq!(display.phase_due(at + Duration::from_millis(100)), None, "told again only after half a second");
    }
}
