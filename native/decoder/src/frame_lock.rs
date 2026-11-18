//! The presentation clock locked to the interface's frames, and the paint
//! points (neoteletext-common's `docs/page-rules.md` RULE-98 and RULE-65).
//!
//! The decoder reads one clock, the engine's microseconds since it started
//! (its "real" time here). The presentation clock every tick, every flash
//! and every animation step is counted on (RULE-4, ANIM-1) is that clock
//! less an offset, which this moves: by at most `PHASE_SLEW_MICROSECONDS`
//! a tick toward the nearest target, `TICK_LEAD_MICROSECONDS` before a
//! composition, while the display can be locked to; and not at all while
//! it cannot, or while the interface's word is stale. Moving the offset
//! moves every clock start counted on it together, so a page's ticks are
//! never skipped or repeated (the rules' `Clock` counts each tick).

use protocol::rules::{
    FramePhase, LOCK_TOLERANCE_PARTS_PER_THOUSAND, PHASE_LOCKED_MICROSECONDS, PHASE_SLEW_MICROSECONDS, PHASE_STALE_MILLISECONDS, TICK_LEAD_MICROSECONDS,
};

/// Microseconds in a millisecond.
const MICROSECONDS_PER_MILLISECOND: u64 = 1_000;
/// Microseconds in a second, for the refresh rate a period gives.
const MICROSECONDS_PER_SECOND: u64 = 1_000_000;
/// The rate every rate a clock locks to is a whole multiple of (RULE-98
/// item 3): the presentation clock's sixty ticks a second.
const LOCK_BASE_HERTZ: u64 = neoteletext_rules::limits::TICKS_PER_SECOND as u64;
/// Parts a thousand, the tolerance's unit.
const PARTS: u64 = 1_000;

/// The interface's compositions as this decoder read them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Compositions {
    /// The refresh period, in microseconds.
    period: u64,
    /// One composition, on the decoder's own clock.
    at: u64,
    /// When the decoder read the message.
    told: u64,
}

/// The lock, the offset and the paint points.
#[derive(Debug, Default)]
pub struct FrameLock {
    compositions: Option<Compositions>,
    /// Microseconds the presentation clock is behind the decoder's own:
    /// negative when ahead.
    offset: i64,
    /// The presentation tick the clock was last moved for: once a tick.
    slewed_for: Option<u32>,
    /// The paint point last painted at, on the decoder's own clock.
    painted: Option<u64>,
    /// Ticks and paint points finished after their composition (RULE-98
    /// item 5).
    late_frames: u32,
    /// The timed test's own setting "the tick left unlocked, with paint
    /// points at the ticks" (RULE-66), for its figures before RULE-98.
    unlocked: bool,
}

/// Whether a display of `period` microseconds can be locked to: its rate
/// a whole multiple of 60 a second within
/// `LOCK_TOLERANCE_PARTS_PER_THOUSAND` (RULE-98 item 3), so 59.7 to 60.3
/// hertz, 119.4 to 120.6 and so on.
#[must_use]
pub fn lockable(period: u64) -> bool {
    let Some(base_period) = MICROSECONDS_PER_SECOND.checked_div(LOCK_BASE_HERTZ) else { return false };
    // The multiple of 60 nearest the rate: 1,000,000 / (period × 60), rounded.
    let Some(multiple) = MICROSECONDS_PER_SECOND.saturating_add(period.saturating_mul(LOCK_BASE_HERTZ) / 2).checked_div(period.saturating_mul(LOCK_BASE_HERTZ))
    else {
        return false;
    };
    if multiple == 0 || period > base_period.saturating_mul(2) {
        return false;
    }
    // |rate − 60n| ≤ 60n × 5 ÷ 1,000, with rate = 1,000,000 ÷ period, times
    // period × 1,000.
    let exact = MICROSECONDS_PER_SECOND.saturating_mul(PARTS);
    let near = LOCK_BASE_HERTZ.saturating_mul(multiple).saturating_mul(period).saturating_mul(PARTS);
    let tolerance = LOCK_BASE_HERTZ.saturating_mul(multiple).saturating_mul(period).saturating_mul(LOCK_TOLERANCE_PARTS_PER_THOUSAND);
    exact.abs_diff(near) <= tolerance
}

/// The presentation tick due at or before `presentation_us`, and when the
/// next is due.
fn next_tick(presentation_us: u64) -> (u32, u64) {
    let tick = neoteletext_core::consts::presentation_tick_at_us(presentation_us);
    let next = tick.saturating_add(1);
    (next, neoteletext_core::consts::presentation_tick_due_us(next))
}

impl FrameLock {
    /// A lock with the timed test's setting: unlocked, the paint points at
    /// the ticks.
    #[must_use]
    pub fn unlocked() -> Self {
        Self { unlocked: true, ..Self::default() }
    }

    /// The presentation clock's reading when the decoder's own reads
    /// `real_us`.
    #[must_use]
    pub const fn presentation_us(&self, real_us: u64) -> u64 {
        real_us.saturating_add_signed(self.offset.saturating_neg())
    }

    /// The decoder's own clock when the presentation clock reads
    /// `presentation_us`.
    const fn real_us(&self, presentation_us: u64) -> u64 {
        presentation_us.saturating_add_signed(self.offset)
    }

    /// The interface told its frames, read at `real_us` (RULE-98 item 2).
    /// When none was in force, a display that can be locked to puts the
    /// next tick on the target at once, without slewing (DEC-6659 item 13).
    pub fn told(&mut self, phase: FramePhase, real_us: u64) {
        let fresh = !self.in_force(real_us);
        self.compositions = Some(Compositions { period: u64::from(phase.period_us), at: phase.composition_after(real_us), told: real_us });
        if fresh && self.locking(real_us) {
            let (_, due) = next_tick(self.presentation_us(real_us));
            let due_real = self.real_us(due);
            // The target at or before the tick, so the clock jumps forward
            // and no tick is counted twice.
            if let Some(target) = self.target_at_or_before(due_real) {
                let distance = i64::try_from(due_real.saturating_sub(target)).unwrap_or(0);
                self.offset = self.offset.saturating_sub(distance);
            }
            self.slewed_for = None;
        }
    }

    /// Whether a `FramePhase` younger than `PHASE_STALE_MILLISECONDS` is in
    /// force (RULE-98 item 4, RULE-65).
    #[must_use]
    pub fn in_force(&self, real_us: u64) -> bool {
        let stale = PHASE_STALE_MILLISECONDS.saturating_mul(MICROSECONDS_PER_MILLISECOND);
        self.compositions.is_some_and(|compositions| real_us.saturating_sub(compositions.told) < stale)
    }

    /// Whether the clock is being locked: a phase in force, of a display
    /// that can be locked to, and not the timed test's unlocked setting.
    #[must_use]
    pub fn locking(&self, real_us: u64) -> bool {
        !self.unlocked && self.in_force(real_us) && self.compositions.is_some_and(|compositions| lockable(compositions.period))
    }

    /// The target, `TICK_LEAD_MICROSECONDS` before a composition, nearest
    /// `real_us`, at or before it.
    fn target_at_or_before(&self, real_us: u64) -> Option<u64> {
        let compositions = self.compositions?;
        let anchor = compositions.at.checked_sub(TICK_LEAD_MICROSECONDS).unwrap_or(compositions.at);
        let period = compositions.period;
        let behind = match real_us.checked_sub(anchor) {
            Some(after) => after.checked_rem(period)?,
            None => period.checked_sub(anchor.checked_sub(real_us)?.checked_rem(period)?)?.checked_rem(period)?,
        };
        real_us.checked_sub(behind)
    }

    /// RULE-98 item 3: before the decoder waits for the next tick, its
    /// clock moves toward the target nearest that tick's due time, the
    /// shorter way, by at most `PHASE_SLEW_MICROSECONDS`, and not at all
    /// within `PHASE_LOCKED_MICROSECONDS`. Once a tick.
    pub fn slew(&mut self, real_us: u64) {
        if !self.locking(real_us) {
            return;
        }
        let (tick, due) = next_tick(self.presentation_us(real_us));
        if self.slewed_for == Some(tick) {
            return;
        }
        self.slewed_for = Some(tick);
        let (Some(compositions), due_real) = (self.compositions, self.real_us(due)) else { return };
        let Some(before) = self.target_at_or_before(due_real) else { return };
        let early = due_real.saturating_sub(before);
        let late = compositions.period.saturating_sub(early);
        // Positive: the tick moves later; negative: earlier.
        let distance = if early <= late { i64::try_from(early).unwrap_or(0).saturating_neg() } else { i64::try_from(late).unwrap_or(0) };
        if distance.unsigned_abs() < PHASE_LOCKED_MICROSECONDS {
            return;
        }
        let slew = i64::try_from(PHASE_SLEW_MICROSECONDS).unwrap_or(0);
        self.offset = self.offset.saturating_add(distance.clamp(slew.saturating_neg(), slew));
    }

    /// The paint point at or before `real_us` (RULE-65): a target of the
    /// interface's compositions while a phase is in force; at the timed
    /// test's unlocked setting, the presentation tick. `None` with no
    /// phase in force: the decoder paints at once.
    pub(crate) fn paint_point_at_or_before(&self, real_us: u64) -> Option<u64> {
        if !self.in_force(real_us) {
            return None;
        }
        if self.unlocked {
            let tick = neoteletext_core::consts::presentation_tick_at_us(self.presentation_us(real_us));
            return Some(self.real_us(neoteletext_core::consts::presentation_tick_due_us(tick)));
        }
        self.target_at_or_before(real_us)
    }

    /// Whether the decoder may paint at `real_us`: always with no phase in
    /// force; otherwise once at each paint point, the first time it wakes
    /// at or after it (RULE-65).
    #[must_use]
    pub fn paint_due(&self, real_us: u64) -> bool {
        self.paint_point_at_or_before(real_us).is_none_or(|point| self.painted != Some(point))
    }

    /// The next paint point after the one last painted, on the
    /// presentation clock, for the loop's deadline: `None` with no phase
    /// in force.
    #[must_use]
    pub fn next_paint_us(&self, real_us: u64) -> Option<u64> {
        let point = self.paint_point_at_or_before(real_us)?;
        if self.painted != Some(point) {
            return Some(self.presentation_us(point));
        }
        let period =
            if self.unlocked { neoteletext_core::consts::presentation_tick_due_us(1) } else { self.compositions.map_or(0, |compositions| compositions.period) };
        Some(self.presentation_us(point.saturating_add(period)))
    }

    /// The decoder painted at the paint point due at `began_us` and
    /// finished at `finished_us`: a frame finished after its composition
    /// (the paint point and the lead) is counted late (RULE-98 item 5).
    pub fn painted(&mut self, began_us: u64, finished_us: u64) {
        let Some(point) = self.paint_point_at_or_before(began_us) else { return };
        self.painted = Some(point);
        if finished_us > point.saturating_add(TICK_LEAD_MICROSECONDS) {
            self.late_frames = self.late_frames.saturating_add(1);
        }
    }

    /// Frames finished after their composition, since the decoder began.
    #[must_use]
    pub const fn late_frames(&self) -> u32 {
        self.late_frames
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::arithmetic_side_effects, clippy::cast_possible_wrap, reason = "tests")]
mod tests {
    use super::*;

    /// A 60 hertz display's period, rounded as an interface would tell it.
    const SIXTY: u32 = 16_667;

    /// How far `due` is from the nearest target of `lock`'s compositions.
    fn from_target(lock: &FrameLock, due: u64) -> i64 {
        let before = lock.target_at_or_before(due).unwrap();
        let period = lock.compositions.unwrap().period;
        let early = (due - before) as i64;
        if early * 2 <= period as i64 { early } else { early - period as i64 }
    }

    /// The real time the next presentation tick after `real` is due at.
    fn next_due(lock: &FrameLock, real: u64) -> (u32, u64) {
        let (tick, due) = next_tick(lock.presentation_us(real));
        (tick, lock.real_us(due))
    }

    /// RULE-98 item 3: rates within five parts a thousand of a multiple of
    /// 60 hertz lock; others do not.
    #[test]
    fn displays_a_whole_multiple_of_sixty_hertz_are_locked_to() {
        // The period of each rate, rounded to the microsecond as the
        // interface tells it.
        let period = |hertz_thousandths: u64| (MICROSECONDS_PER_SECOND * 1_000 + hertz_thousandths / 2) / hertz_thousandths;
        for hertz_thousandths in [59_700_u64, 59_940, 60_000, 60_300, 119_400, 120_000, 120_600, 180_000, 240_000] {
            assert!(lockable(period(hertz_thousandths)), "{hertz_thousandths}");
        }
        for hertz_thousandths in [50_000_u64, 59_600, 60_400, 75_000, 100_000, 144_000, 30_000] {
            assert!(!lockable(period(hertz_thousandths)), "{hertz_thousandths}");
        }
    }

    /// Runs a page's clock (the rules' own `Clock`, started at the next
    /// tick) for `ticks` ticks against a display of `period` microseconds
    /// whose compositions fall at `first_composition` and every period
    /// after, the interface telling its phase at every tick. Each wake at a
    /// tick's due time must run exactly that one tick; the clock may move
    /// at most `PHASE_SLEW_MICROSECONDS` a tick. Gives each tick's distance
    /// from its nearest target.
    fn run_ticks(lock: &mut FrameLock, period: u64, first_composition: u64, ticks: usize) -> Vec<u64> {
        let composition = |real: u64| first_composition + (real.saturating_sub(first_composition) / period + 1) * period;
        let mut real = first_composition.saturating_sub(period);
        let tell = |lock: &mut FrameLock, real: u64| lock.told(FramePhase::at(u32::try_from(period).unwrap(), composition(real), real).unwrap(), real);
        tell(lock, real);
        let (_, start) = next_tick(lock.presentation_us(real));
        let mut clock = neoteletext_rules::clock::Clock::new();
        let mut distances = Vec::new();
        for _ in 0..ticks {
            let before = lock.offset;
            lock.slew(real);
            assert!((lock.offset - before).unsigned_abs() <= PHASE_SLEW_MICROSECONDS, "at most 100 a tick");
            real = lock.real_us(start + clock.next_due());
            assert_eq!(clock.wake(lock.presentation_us(real) - start), 1, "each tick once, none skipped or repeated");
            distances.push(from_target(lock, real).unsigned_abs());
            tell(lock, real);
        }
        distances
    }

    /// RULE-98 item 3: at 60 hertz the ticks come to rest 4,000
    /// microseconds before each composition within 200, the clock having
    /// moved no more than 100 in a tick; the first phase puts the next
    /// tick on its target at once.
    #[test]
    fn at_sixty_hertz_the_ticks_come_to_rest_before_each_composition() {
        let mut lock = FrameLock::default();
        lock.told(FramePhase { period_us: SIXTY, next_in_us: 9_000 }, 1_000_000);
        let (_, due) = next_due(&lock, 1_000_000);
        assert_eq!(from_target(&lock, due), 0, "the first phase puts the next tick on its target");
        // The display then a quarter-period later: slewed toward.
        let distances = run_ticks(&mut lock, u64::from(SIXTY), 2_000_000 + 4_000, 200);
        assert!(distances.first().is_some_and(|first| *first > 1_000), "it began away from its target: {distances:?}");
        assert!(distances.iter().skip(150).all(|distance| *distance < PHASE_LOCKED_MICROSECONDS), "{distances:?}");
    }

    /// RULE-98 item 3: at 120 hertz every tick is 4,000 before a
    /// composition.
    #[test]
    fn at_one_hundred_and_twenty_hertz_every_tick_is_before_a_composition() {
        let mut lock = FrameLock::default();
        let distances = run_ticks(&mut lock, 8_333, 5_001_234, 120);
        assert!(distances.iter().all(|distance| *distance < PHASE_LOCKED_MICROSECONDS), "{distances:?}");
    }

    /// RULE-98 item 3: a display of 59.94 hertz is followed for ten minutes
    /// with no tick skipped or repeated, every tick near its target.
    #[test]
    fn a_display_of_fifty_nine_point_nine_four_hertz_is_followed() {
        // 59.94 hertz: 16,683 microseconds, on the display's own clock.
        let mut lock = FrameLock::default();
        let distances = run_ticks(&mut lock, 16_683, 1_000, 10 * 60 * 60);
        let worst = distances.iter().skip(120).max().copied().unwrap();
        assert!(worst < PHASE_LOCKED_MICROSECONDS + PHASE_SLEW_MICROSECONDS, "the ticks stay on their targets: {worst}");
    }

    /// RULE-98 item 4: 144 hertz and a stale phase leave the clock free.
    #[test]
    fn a_display_it_cannot_lock_to_and_a_stale_phase_leave_the_clock_free() {
        let mut lock = FrameLock::default();
        lock.told(FramePhase { period_us: 6_944, next_in_us: 3_000 }, 1_000);
        for real in (1_000..1_000_000).step_by(16_667) {
            lock.slew(real);
        }
        assert_eq!(lock.offset, 0, "144 hertz is not locked to");
        let mut lock = FrameLock::default();
        lock.told(FramePhase { period_us: SIXTY, next_in_us: 3_000 }, 1_000);
        let offset = lock.offset;
        let stale = 1_000 + PHASE_STALE_MILLISECONDS * MICROSECONDS_PER_MILLISECOND;
        for real in (stale..stale + 1_000_000).step_by(16_667) {
            lock.slew(real);
        }
        assert_eq!(lock.offset, offset, "a stale phase moves nothing");
        assert!(lock.paint_due(stale) && lock.next_paint_us(stale).is_none(), "and paints at once");
    }

    /// RULE-65: with a phase in force a paint point is every target, on a
    /// display locked to or not; each is painted once; a paint finished
    /// after its composition is counted late.
    #[test]
    fn paint_points_are_four_milliseconds_before_each_composition() {
        let mut lock = FrameLock::default();
        // 144 hertz: not locked, paint points still follow it.
        lock.told(FramePhase { period_us: 6_944, next_in_us: 6_000 }, 100_000);
        let first = 100_000 + 6_000 - 4_000;
        assert_eq!(lock.paint_point_at_or_before(first - 1), Some(first - 6_944), "the paint point before");
        assert_eq!(lock.paint_point_at_or_before(first), Some(first));
        assert!(lock.paint_due(first));
        lock.painted(first, first + 1_000);
        assert!(!lock.paint_due(first + 3_000), "one paint at a paint point");
        assert_eq!(lock.next_paint_us(first + 3_000).map(|at| lock.real_us(at)), Some(first + 6_944));
        assert_eq!(lock.late_frames(), 0);
        lock.painted(first + 6_944, first + 6_944 + 4_001);
        assert_eq!(lock.late_frames(), 1, "finished after its composition");
    }

    /// RULE-66: the timed test's unlocked setting never moves the clock
    /// and paints at the ticks.
    #[test]
    fn unlocked_the_paint_points_are_the_ticks() {
        let mut lock = FrameLock::unlocked();
        lock.told(FramePhase { period_us: SIXTY, next_in_us: 9_000 }, 1_000_000);
        for real in (1_000_000..2_000_000).step_by(16_667) {
            lock.slew(real);
        }
        assert_eq!(lock.offset, 0);
        let tick_due = neoteletext_core::consts::presentation_tick_due_us(neoteletext_core::consts::presentation_tick_at_us(1_500_000));
        assert_eq!(lock.paint_point_at_or_before(1_500_000), Some(tick_due));
    }
}
