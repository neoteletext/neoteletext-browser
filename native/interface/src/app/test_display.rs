//! The timed test's display (neoteletext-common's `docs/page-rules.md`
//! RULE-66), only in the build the test makes (the feature `timed-test`).
//!
//! The test runs the browser on a private X display, which composes
//! nothing and has no refresh: the interface presents each frame the
//! moment it draws it, and the platform gives it no frame times. For the
//! lock's figures the test needs a display that composes at a refresh
//! rate: this one composes at 60 hertz on the interface's own clock, as a
//! compositor does on the platforms the interface ships for (`frame.rs`):
//! the interface draws and presents a frame when it is asked for, and the
//! display shows it at its first composition after the present finished.
//! The time the test records for a frame is that composition's.
//!
//! An earlier form drew a frame asked for only at the next composition,
//! and recorded the end of its present: that put the interface's whole
//! drawing after the composition it was meant for, which no platform does,
//! and missed a composition whenever the event loop woke more than a
//! millisecond after one (DEC-6659 item 23).

use std::time::{Duration, Instant};

use protocol::rules::FramePhase;

/// The test display's refresh period: 60 hertz, rounded to the
/// microsecond.
const PERIOD_MICROSECONDS: u32 = 16_667;
/// How often the phase is told again: twice within the time a decoder
/// keeps it (RULE-98 item 4's `PHASE_STALE_MILLISECONDS`), so it is never
/// stale while the window is open.
const TELL_EVERY: Duration = Duration::from_millis(protocol::rules::PHASE_STALE_MILLISECONDS / 2);

/// The display's compositions.
#[derive(Debug)]
pub(super) struct TestDisplay {
    epoch: Instant,
    /// When the phase was last told.
    told: Option<Instant>,
    /// The test's setting "paced display" (DEC-6659 item 25): a frame
    /// asked for is drawn at the next composition, as Wayland's frame
    /// callback paces winit's redraws, and the interface tells the phase
    /// its own redraws give rather than this display's.
    paced: bool,
    /// On the paced display, the composition a frame asked for waits for.
    owed: Option<Instant>,
}

impl TestDisplay {
    pub(super) fn new() -> Self {
        Self { epoch: Instant::now(), told: None, paced: protocol::timed_test::setting(protocol::timed_test::PACED_DISPLAY), owed: None }
    }

    /// Whether this is the paced display.
    pub(super) const fn paced(&self) -> bool {
        self.paced
    }

    /// A frame was asked for: whether it waits for the next composition,
    /// as on the paced display.
    pub(super) fn defer(&mut self) -> bool {
        if self.paced && self.owed.is_none() {
            let now = Instant::now();
            // A composition this moment is the one already past.
            self.owed = Some(self.next_composition(now.checked_add(Duration::from_micros(1)).unwrap_or(now)));
        }
        self.paced
    }

    /// Whether the composition a frame waits for has come, on the paced
    /// display: the frame is then asked for, however late the loop woke.
    pub(super) fn composition_reached(&mut self, now: Instant) -> bool {
        match self.owed {
            Some(at) if now >= at => {
                self.owed = None;
                true
            }
            _ => false,
        }
    }

    /// The first composition at or after `now`: the one that shows a frame
    /// whose present finished at `now`.
    pub(super) fn next_composition(&self, now: Instant) -> Instant {
        let since = u64::try_from(now.saturating_duration_since(self.epoch).as_micros()).unwrap_or(u64::MAX);
        let period = u64::from(PERIOD_MICROSECONDS);
        let compositions = since.div_ceil(period);
        self.epoch.checked_add(Duration::from_micros(compositions.saturating_mul(period))).unwrap_or(now)
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

    /// When the loop must wake for the display: the next telling of the
    /// phase, and the composition a frame waits for.
    pub(super) fn wake(&self, now: Instant) -> Instant {
        let tell = self.told.and_then(|told| told.checked_add(TELL_EVERY)).unwrap_or(now);
        self.owed.map_or(tell, |owed| owed.min(tell))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::arithmetic_side_effects, reason = "a test")]
mod tests {
    use super::*;

    fn period() -> Duration {
        Duration::from_micros(u64::from(PERIOD_MICROSECONDS))
    }

    /// RULE-66: compositions every 16,667 microseconds; a frame presented
    /// between two is shown at the second; the phase told is the time to
    /// the next.
    #[test]
    fn the_test_display_composes_at_sixty_hertz() {
        let mut display = TestDisplay::new();
        assert!(!display.paced(), "the paced display only by the test's setting");
        let at = display.epoch + Duration::from_micros(5_000);
        assert_eq!(display.next_composition(at), display.epoch + period());
        assert_eq!(display.next_composition(display.epoch + period()), display.epoch + period(), "a present that ends on a composition is in it");
        assert_eq!(display.next_composition(display.epoch + period() + Duration::from_micros(1)), display.epoch + period() * 2);
        let phase = display.phase_due(at).unwrap();
        assert_eq!(phase.period_us, PERIOD_MICROSECONDS);
        assert_eq!(phase.next_in_us, PERIOD_MICROSECONDS - 5_000);
        assert_eq!(display.phase_due(at + Duration::from_millis(100)), None, "told again only after half a second");
    }
}
