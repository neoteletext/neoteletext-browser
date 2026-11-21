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

// DEC-7031 items 2 and 4: the ReGIS renderer watches as many drawings as
// one rule table's `painted` triggers may name.
const _: () = assert!(neoteletext_rules::limits::MAX_PAINT_WATCHED == regis::limits::MAX_WATCHED_PAINTINGS);
// DEC-7031 items 2 and 3: a stream paced at a line's speed is held to the
// same number of bytes in either renderer, so a replay lasts no longer in
// one language than in the other.
const _: () = assert!(regis::limits::MAX_REPLAY_HELD_BYTES == naplps::limits::MAX_WAIT_HELD_BYTES);
// DEC-7031 item 1: a PAINT record's default rate is a rate the ReGIS
// renderer takes as it is given.
const _: () = assert!(neoteletext_web::consts::MAX_DEFAULT_PAINT_RATE == regis::limits::MAX_PAINT_RATE);

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

/// Whether the "painted" event of a sub-page's whole picture is owed
/// (DEC-7031 items 4 and 5, REQ-7026): the open page's rule table has a
/// `painted` trigger that names the page, bytes of the picture's stream
/// were drawn since the event last fired, and nothing of the stream waits
/// to be drawn.
#[derive(Clone, Copy, Debug, Default)]
pub struct WholePicture {
    /// The rule table of this sub-page names the page's whole picture in
    /// a `painted` trigger.
    wanted: bool,
    /// Bytes of the stream were given to the renderer's replay or player
    /// since the event last fired, or the table that wants the event was
    /// loaded after bytes were.
    drew: bool,
    /// The presentation tick the event last fired at: it fires at most
    /// once a tick.
    fired: Option<u32>,
}

impl WholePicture {
    /// The table that names the page's picture was loaded, or dropped.
    /// Loaded when bytes of the stream have come already (`begun`), the
    /// event is owed for the picture as it stands: a channel sends the
    /// stream before the table (DEC-7031, "what it makes more exact").
    pub const fn watch(&mut self, wanted: bool, begun: bool) {
        self.wanted = wanted;
        self.drew = wanted && (self.drew || begun);
    }

    /// Bytes of the stream were given to the replay or the player.
    pub const fn drew(&mut self) {
        self.drew = true;
    }

    /// Whether the event is owed now, `idle` saying that the replay or the
    /// player has nothing left to draw.
    #[must_use]
    pub const fn due(&self, idle: bool) -> bool {
        self.wanted && self.drew && idle
    }

    /// Takes the event at presentation tick `tick` if it is owed and none
    /// fired in that tick; one owed in a tick that had one waits for the
    /// next.
    pub fn take(&mut self, idle: bool, tick: u32) -> bool {
        if !self.due(idle) || self.fired == Some(tick) {
            return false;
        }
        self.drew = false;
        self.fired = Some(tick);
        true
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
