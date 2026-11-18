//! Animation in the decoder (neoteletext-common's `docs/animation.md`,
//! ANIM-22): the decoder is the compositor. It keeps the presentation
//! clock (ANIM-1), each shown sub-page's animation state (ANIM-6) and
//! gives it to that sub-page's renderer, advances frame cycles (ANIM-14),
//! the ReGIS paint rate (ANIM-15) and NAPLPS WAIT and BLINK (ANIM-20) on
//! the clock, holds atomic frames (ANIM-2) and the animation budget
//! (ANIM-16).
//!
//! What is here is shared by the three renderers' parts: the tick of the
//! clock, the next tick a cycle changes its frame at, and the page's
//! picture as a rule table sees it ([`PagePicture`]).

use neoteletext_core::consts::{presentation_tick_at_us, presentation_tick_due_us};
use neoteletext_rules::animate::{AnimationState, Cycle};
use neoteletext_rules::table::Target;

pub mod frames;
pub mod picture;
pub mod teletext;

pub use picture::PagePicture;

// ANIM-1: the engine core's presentation clock and the rule table's ticks
// are one clock's, as the documents say; the decoder passes ticks between
// the two without converting.
const _: () = assert!(neoteletext_core::consts::PRESENTATION_TICKS_PER_SECOND == neoteletext_rules::limits::TICKS_PER_SECOND);
// ANIM-16: every renderer lets the same number of a page's objects cycle.
const _: () = assert!(regis::limits::MAX_CYCLING_OBJECTS == naplps::limits::MAX_CYCLING_OBJECTS);
const _: () = assert!(regis::limits::MAX_CYCLING_OBJECTS == neoteletext_core::limits::MAX_CYCLING_OBJECTS);

/// The presentation clock's tick due at `now_us` of the decoder's clock
/// (ANIM-1): tick `n` is due `n × 50,000 ÷ 3` microseconds after the
/// decoder started.
#[must_use]
pub const fn tick_at(now_us: u64) -> u32 {
    presentation_tick_at_us(now_us)
}

/// When the tick after `tick` is due, on the decoder's clock.
#[must_use]
pub const fn due_after(tick: u32) -> u64 {
    presentation_tick_due_us(tick.saturating_add(1))
}

/// When tick `tick` is due, on the decoder's clock.
#[must_use]
pub const fn due_at(tick: u32) -> u64 {
    presentation_tick_due_us(tick)
}

/// The first tick after `tick` at which `cycle` shows another frame
/// (ANIM-14), or `None` when it never will: a cycle that runs once and has
/// reached its last frame. A decoder wakes for that tick and no sooner
/// (ANIM-1).
#[must_use]
pub fn next_cycle_change(cycle: &Cycle, tick: u32) -> Option<u32> {
    if cycle.frame_at(tick).1 {
        return None;
    }
    if tick < cycle.start {
        return Some(cycle.start);
    }
    let period = u32::from(cycle.period).max(1);
    let steps = tick.saturating_sub(cycle.start).checked_div(period).unwrap_or(0);
    Some(cycle.start.saturating_add(steps.saturating_add(1).saturating_mul(period)))
}

/// The first tick after `tick` at which any cycle of `entries` shows
/// another frame.
#[must_use]
pub fn next_cycle_tick(entries: &[(Target, AnimationState)], tick: u32) -> Option<u32> {
    entries.iter().filter_map(|(_, state)| state.cycle.as_ref()).filter_map(|cycle| next_cycle_change(cycle, tick)).min()
}

/// What a sub-page's animation has cost the viewer, for the page's rule
/// table host to report (ANIM-16, RULE-48).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    /// Repaints for animation that were held back for a later tick: each
    /// is one `RULES_FAULT` of fault 8, the animation slowed.
    pub slowed: u32,
    /// Operations that did nothing: on a target that does not exist, on
    /// one more object than may hold state, or a cycle more than may run.
    pub skipped: u32,
}

impl Counters {
    /// Adds another sub-page's counts.
    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        Self { slowed: self.slowed.saturating_add(other.slowed), skipped: self.skipped.saturating_add(other.skipped) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ANIM-14, ANIM-1: a looping cycle next changes at each multiple of
    /// its period from its start; a cycle that runs once never changes
    /// after its last frame.
    #[test]
    fn a_cycle_next_changes_at_its_periods_and_never_after_its_end() {
        let looping = Cycle { first: 0, last: 3, period: 6, once: false, start: 10 };
        assert_eq!(next_cycle_change(&looping, 0), Some(10), "before its start");
        assert_eq!(next_cycle_change(&looping, 10), Some(16));
        assert_eq!(next_cycle_change(&looping, 15), Some(16));
        assert_eq!(next_cycle_change(&looping, 16), Some(22));
        let once = Cycle { first: 1, last: 2, period: 5, once: true, start: 0 };
        assert_eq!(next_cycle_change(&once, 4), Some(5));
        assert_eq!(next_cycle_change(&once, 5), None, "the last frame is reached");
    }

    /// ANIM-1: tick `n` is due `n × 50,000 ÷ 3` microseconds after the start.
    #[test]
    fn ticks_are_sixtieths_of_a_second() {
        assert_eq!(tick_at(0), 0);
        assert_eq!(tick_at(16_666), 1);
        assert_eq!(tick_at(1_000_000), 60);
        assert_eq!(due_at(60), 1_000_000);
        assert_eq!(due_after(59), 1_000_000);
    }
}
