//! The display's frames as the interface's own presentation shows them
//! (neoteletext-common's `docs/page-rules.md` RULE-98 item 1; DEC-6659
//! item 25).
//!
//! winit gives no frame times on any platform, so the interface derives
//! them: the refresh period from the refresh rate winit reports for the
//! window's monitor, or where it reports none the shortest interval
//! between its recent frames, and the phase from the moments the window system has
//! it draw its frames, which it then hands back. Those moments say where
//! the display's compositions are only on a platform that paces them by
//! the display, as Wayland's frame callback does for winit's redraws: a
//! frame asked for is drawn just after a composition. Where frames are presented whenever they are
//! drawn (X11 without a compositor's pacing, a Windows `BitBlt`, a macOS
//! layer's contents), the moments fall anywhere in the period, and a phase
//! taken from them would put the decoder's paint points at no particular
//! place before the display's frames, delaying a picture by up to a period
//! for nothing. So the phase is told only once the presents have come at
//! whole periods from one another, within `PHASE_REPORT_MICROSECONDS`, for
//! `PACED_PRESENTS` presents running; otherwise the interface tells none,
//! and by RULE-65 the decoder paints at once.

use std::time::{Duration, Instant};

use protocol::rules::{FramePhase, LEAST_REFRESH_MICROSECONDS, MOST_REFRESH_MICROSECONDS, PHASE_REPORT_MICROSECONDS, PHASE_STALE_MILLISECONDS};

/// How many presents running must fall on the display's period before
/// their phase is believed: eight, so that frames drawn whenever they are
/// asked for, falling anywhere in a 16.7 millisecond period, pass by chance
/// within half a millisecond of it less than once in 10^11 runs of eight
/// (`(2 × 0.5 ÷ 16.7)^7`).
pub(super) const PACED_PRESENTS: u32 = 8;
/// How often the phase is told again while presents keep it: twice within
/// the time a decoder keeps it (RULE-98 item 4), so that it is never stale
/// on a steady display (DEC-6659 item 14).
const TELL_EVERY: Duration = Duration::from_millis(PHASE_STALE_MILLISECONDS / 2);
/// Millihertz in a hertz, times microseconds in a second: a refresh rate
/// in millihertz divides this into its period in microseconds.
const MILLIHERTZ_MICROSECONDS: u64 = 1_000 * 1_000_000;
/// How many intervals between presents the period is measured from where
/// the monitor reports no rate: the shortest of the last sixteen, which,
/// while anything animates, is one period of a paced display.
const MEASURED_INTERVALS: usize = 16;

/// The display's period and the presents that place its frames.
#[derive(Debug, Default)]
pub(super) struct PresentPhase {
    /// The refresh period, in microseconds, from the monitor's rate.
    rate_us: Option<u32>,
    /// The intervals between the last presents, in microseconds, oldest
    /// first, for the period where the monitor reports no rate.
    intervals: std::collections::VecDeque<u32>,
    /// The refresh period in use: the monitor's, else the one measured.
    period_us: Option<u32>,
    /// The last present, which lies on a composition while they are paced.
    anchor: Option<Instant>,
    /// Presents running that fell on the period.
    paced: u32,
    /// When the phase was last told, and the composition it then foretold.
    told: Option<(Instant, Instant)>,
}

/// The period in microseconds of a refresh rate in millihertz.
pub(super) fn period_of(millihertz: u32) -> Option<u32> {
    MILLIHERTZ_MICROSECONDS.checked_div(u64::from(millihertz)).and_then(|period| u32::try_from(period).ok())
}

impl PresentPhase {
    /// The monitor's refresh rate, in millihertz, as winit reports it: a
    /// new period starts the pacing again.
    pub(super) fn set_rate(&mut self, millihertz: Option<u32>) {
        self.rate_us = millihertz.and_then(period_of);
        self.choose_period();
    }

    /// The period in use: the monitor's rate, else the shortest interval
    /// between recent presents that a refresh period can be (RULE-98
    /// item 1's bounds); a new period starts the pacing again.
    fn choose_period(&mut self) {
        let measured = self
            .intervals
            .iter()
            .copied()
            .filter(|interval| (LEAST_REFRESH_MICROSECONDS..=MOST_REFRESH_MICROSECONDS).contains(interval))
            .min()
            .filter(|_| self.intervals.len() >= MEASURED_INTERVALS);
        let period = self.rate_us.or(measured);
        // A measured period moves by the jitter of the presents: only a
        // move past what the pacing allows is a new period.
        let moved = match (period, self.period_us) {
            (Some(new), Some(old)) => u64::from(new.abs_diff(old)) > PHASE_REPORT_MICROSECONDS,
            (new, old) => new != old,
        };
        if moved {
            self.period_us = period;
            self.paced = 0;
            self.told = None;
        }
    }

    /// How far `at` is from the nearest composition the anchor foretells,
    /// in microseconds.
    fn distance(&self, at: Instant) -> Option<u64> {
        let (period, anchor) = (u64::from(self.period_us?), self.anchor?);
        let since = u64::try_from(at.checked_duration_since(anchor)?.as_micros()).ok()?;
        let into = since.checked_rem(period)?;
        Some(into.min(period.saturating_sub(into)))
    }

    /// A frame was handed to the window system at `at`.
    pub(super) fn presented(&mut self, at: Instant) {
        if let Some(interval) = self.anchor.and_then(|anchor| at.checked_duration_since(anchor)) {
            if self.intervals.len() >= MEASURED_INTERVALS {
                self.intervals.pop_front();
            }
            self.intervals.push_back(u32::try_from(interval.as_micros()).unwrap_or(u32::MAX));
            self.choose_period();
        }
        self.paced = match self.distance(at) {
            Some(distance) if distance <= PHASE_REPORT_MICROSECONDS => self.paced.saturating_add(1),
            _ => 0,
        };
        self.anchor = Some(at);
    }

    /// The next composition after `now`, while the presents are paced and
    /// fresh: none once no present has come for `PHASE_STALE_MILLISECONDS`.
    fn next_composition(&self, now: Instant) -> Option<Instant> {
        let (period, anchor) = (u64::from(self.period_us?), self.anchor?);
        if self.paced < PACED_PRESENTS || now.saturating_duration_since(anchor) >= Duration::from_millis(PHASE_STALE_MILLISECONDS) {
            return None;
        }
        let since = u64::try_from(now.saturating_duration_since(anchor).as_micros()).ok()?;
        let periods = since.checked_div(period)?.saturating_add(1);
        anchor.checked_add(Duration::from_micros(periods.saturating_mul(period)))
    }

    /// The phase to tell now (RULE-98 item 1): when first known, when the
    /// composition last foretold is wrong by more than
    /// `PHASE_REPORT_MICROSECONDS`, and every half second.
    pub(super) fn phase_due(&mut self, now: Instant) -> Option<FramePhase> {
        let next = self.next_composition(now)?;
        let period = self.period_us?;
        let due = match self.told {
            None => true,
            Some((told, foretold)) => {
                let period = Duration::from_micros(u64::from(period));
                // The composition the last message foretold, carried to the
                // one nearest the next.
                let mut carried = foretold;
                while carried.checked_add(period).is_some_and(|later| later <= next) {
                    carried = carried.checked_add(period).unwrap_or(next);
                }
                let wrong = next
                    .saturating_duration_since(carried)
                    .min(carried.checked_add(period).map_or(Duration::ZERO, |after| after.saturating_duration_since(next)));
                now.saturating_duration_since(told) >= TELL_EVERY || wrong > Duration::from_micros(PHASE_REPORT_MICROSECONDS)
            }
        };
        if !due {
            return None;
        }
        self.told = Some((now, next));
        FramePhase::at(period, u64::try_from(next.saturating_duration_since(now).as_micros()).unwrap_or(0), 0)
    }

    /// When the loop must wake to tell the phase again, while it is told.
    pub(super) fn wake(&self) -> Option<Instant> {
        self.told.and_then(|(told, _)| told.checked_add(TELL_EVERY))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::arithmetic_side_effects, reason = "a test")]
mod tests {
    use super::*;

    /// 60 hertz as winit reports it, in millihertz.
    const SIXTY_MILLIHERTZ: u32 = 60_000;

    /// DEC-6659 item 25: presents paced by a 60 hertz display, each a few
    /// hundred microseconds after its composition, give its phase once
    /// `PACED_PRESENTS` have come, and the composition told is within the
    /// presents' own lateness of the true one.
    #[test]
    fn paced_presents_tell_the_displays_phase() {
        let mut phase = PresentPhase::default();
        phase.set_rate(Some(SIXTY_MILLIHERTZ));
        assert_eq!(period_of(SIXTY_MILLIHERTZ), Some(16_666));
        let epoch = Instant::now();
        let composition = |n: u64| epoch + Duration::from_micros(n * 16_666);
        let late = [120_u64, 300, 80, 250, 400, 150, 90, 310, 200];
        for (n, after) in late.iter().enumerate() {
            let n = u64::try_from(n).unwrap();
            assert_eq!(phase.phase_due(composition(n) + Duration::from_micros(*after)), None, "not before {PACED_PRESENTS} presents running");
            phase.presented(composition(n) + Duration::from_micros(*after));
        }
        let now = composition(8) + Duration::from_micros(5_000);
        let told = phase.phase_due(now).unwrap();
        assert_eq!(told.period_us, 16_666);
        let foretold = now + Duration::from_micros(u64::from(told.next_in_us));
        let error = foretold.saturating_duration_since(composition(9)).max(composition(9).saturating_duration_since(foretold));
        assert!(error <= Duration::from_micros(400), "within the presents' lateness: {error:?}");
        assert_eq!(phase.phase_due(now + Duration::from_millis(100)), None, "told again only after half a second");
    }

    /// Frames presented whenever they are drawn, anywhere in the period,
    /// give no phase: the decoder then paints at once (RULE-65).
    #[test]
    fn presents_at_any_moment_tell_nothing() {
        let mut phase = PresentPhase::default();
        phase.set_rate(Some(SIXTY_MILLIHERTZ));
        let epoch = Instant::now();
        let mut at = epoch;
        for gap in [21_000_u64, 7_300, 40_100, 13_900, 25_500, 9_100, 31_700, 18_800, 44_400, 12_200, 27_000, 5_500] {
            at += Duration::from_micros(gap);
            phase.presented(at);
            assert_eq!(phase.phase_due(at + Duration::from_micros(10)), None);
        }
    }

    /// With no rate from the monitor the period is measured from the
    /// presents, once sixteen intervals are held, some of them a frame
    /// skipped; a present of a second ago is stale.
    #[test]
    fn with_no_rate_the_period_is_measured_and_a_stale_present_tells_nothing() {
        let mut phase = PresentPhase::default();
        phase.set_rate(None);
        let epoch = Instant::now();
        let frames = [0_u64, 1, 2, 4, 5, 6, 7, 9, 10, 11, 12, 13, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28];
        for &n in &frames {
            phase.presented(epoch + Duration::from_micros(n * 16_667 + n % 3 * 40));
        }
        let last = epoch + Duration::from_micros(28 * 16_667);
        let told = phase.phase_due(last + Duration::from_millis(1)).unwrap();
        assert!(told.period_us.abs_diff(16_667) <= 100, "the period measured: {}", told.period_us);
        assert_eq!(phase.phase_due(last + Duration::from_millis(PHASE_STALE_MILLISECONDS + 1)), None, "stale");
    }
}
