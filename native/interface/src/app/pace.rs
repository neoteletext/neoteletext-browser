//! When the window at hand draws its next frame (decision 53, requirement
//! 18.11, REQ-RCH-001 of `~/devel/neoteletext-common/RETURN-CHANNEL.md`):
//! a page's frame the viewer is waiting for at once, what else the trusted
//! core changed at a pace, and what animates at the pace it asks for; never
//! while nothing changes.

/// What the window at hand has drawn and has still to draw, in
/// milliseconds from the interface's start.
#[derive(Debug, Default)]
pub(super) struct Pace {
    /// When the last frame that changed the window was presented: what the
    /// trusted core changes is paced from it, so a frame that changed
    /// nothing delays nothing.
    presented: u64,
    /// When the last frame was drawn, whether it changed the window or
    /// not: what animates is paced from it, so a frame that changed
    /// nothing is not drawn again at once.
    drawn: u64,
    /// The trusted core changed something not yet drawn.
    dirty: bool,
    /// When the first change not yet drawn came.
    dirty_since: u64,
    /// A frame of the page the viewer is typing into or pointing at
    /// arrived and is not yet drawn.
    awaited: bool,
}

/// What the window should do about its next frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Next {
    /// Ask for a frame now.
    pub draw: bool,
    /// When to wake to look again, in milliseconds from the interface's
    /// start; none while nothing waits to be drawn or animates.
    pub wake: Option<u64>,
}

impl Pace {
    /// The trusted core changed something the window shows, at `now`:
    /// drawn with the next paced frame, as pages load and flash many times
    /// a second and one frame shows all that arrived since the last.
    pub fn changed(&mut self, now: u64) {
        if !self.dirty {
            self.dirty_since = now;
        }
        self.dirty = true;
    }

    /// A frame of the page the viewer is typing into or pointing at
    /// arrived: drawn at once, whatever was drawn just before it, such as
    /// the frame for the acknowledgement that came a millisecond or two
    /// ahead of the answer. The trusted core sends a page's frames at most
    /// a few times a field (finding F-11), which bounds these frames.
    pub fn awaited(&mut self) {
        self.dirty = true;
        self.awaited = true;
    }

    /// A frame was drawn at `now`; `presented` when it changed the window.
    pub fn drawn(&mut self, now: u64, presented: bool) {
        self.drawn = now;
        if presented {
            self.presented = now;
        }
        self.dirty = false;
        self.awaited = false;
    }

    /// Whether to draw at `now`, and when to look again: `animation` is
    /// the interval the browser's animations ask for, `change` the least
    /// interval between frames for what the trusted core changed, and
    /// `hold` how long such a change waits for the awaited page's frame
    /// to come and be drawn with it (REQ-6400): none while no channel is
    /// typed into.
    pub fn next(&self, now: u64, animation: Option<u64>, change: u64, hold: u64) -> Next {
        if self.awaited {
            return Next { draw: true, wake: None };
        }
        let animated = animation.map(|interval| self.drawn.saturating_add(interval));
        let changed = self.dirty.then(|| self.presented.saturating_add(change).max(self.dirty_since.saturating_add(hold)));
        let due = animated.into_iter().chain(changed).min();
        Next { draw: due.is_some_and(|due| now >= due), wake: due }
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, reason = "a test's clock")]
mod tests {
    use super::*;

    /// One frame of a 60 hertz display, as the window paces what a channel
    /// being typed into pushes.
    const CHANGE: u64 = 16;
    /// How long after the acknowledgement the channel's answer comes on
    /// the local server (`RETURN-CHANNEL.md` section 2.1).
    const ANSWER_AFTER: u64 = 2;
    /// REQ-RCH-001's bound: the answer drawn within this long of its
    /// arrival.
    const WITHIN: u64 = 2;

    /// The frame that would be drawn at or after `from`, looking every
    /// millisecond as the event loop would when woken.
    fn drawn_at(pace: &Pace, from: u64, animation: Option<u64>) -> Option<u64> {
        (from..from + 1_000).find(|&now| pace.next(now, animation, CHANGE, 0).draw)
    }

    /// REQ-RCH-001: the acknowledgement's frame is drawn, and the answer,
    /// arriving two milliseconds later, is drawn within two milliseconds
    /// of its arrival, not a display frame after the acknowledgement's.
    #[test]
    fn the_answer_after_an_acknowledgement_is_drawn_as_it_arrives() {
        let mut pace = Pace::default();
        let acknowledged = 1_000;
        pace.changed(0);
        assert_eq!(drawn_at(&pace, acknowledged, None), Some(acknowledged), "the acknowledgement's frame");
        pace.drawn(acknowledged, true);
        let answered = acknowledged + ANSWER_AFTER;
        pace.awaited();
        let drawn = drawn_at(&pace, answered, None).expect("the answer is drawn");
        assert!(drawn - answered <= WITHIN, "the answer drawn {} ms after it arrived", drawn - answered);
    }

    /// REQ-RCH-001: a frame that changed nothing delays nothing; the next
    /// change is drawn as it comes.
    #[test]
    fn a_frame_that_changed_nothing_does_not_delay_the_next() {
        let mut pace = Pace::default();
        let start = 1_000;
        pace.changed(0);
        pace.drawn(start, true);
        pace.changed(0);
        pace.drawn(start + CHANGE, false);
        pace.changed(0);
        assert_eq!(drawn_at(&pace, start + CHANGE + 1, None), Some(start + CHANGE + 1), "not held a frame after the one that changed nothing");
    }

    /// What the trusted core changes is still paced after a frame that
    /// changed the window, so many changes share one frame.
    #[test]
    fn changes_are_paced_from_the_last_frame_presented() {
        let mut pace = Pace::default();
        let start = 1_000;
        pace.changed(0);
        pace.drawn(start, true);
        pace.changed(0);
        assert_eq!(pace.next(start + 1, None, CHANGE, 0), Next { draw: false, wake: Some(start + CHANGE) });
        assert_eq!(drawn_at(&pace, start + 1, None), Some(start + CHANGE));
    }

    /// Decision 53: nothing is drawn while nothing changes or animates, and
    /// an animation's frame that changed nothing is not drawn again at
    /// once: the loop sleeps until the animation's next frame.
    #[test]
    fn nothing_is_drawn_while_nothing_changes() {
        const ANIMATION: u64 = 33;
        let mut pace = Pace::default();
        let start = 1_000;
        assert_eq!(pace.next(start, None, CHANGE, 0), Next { draw: false, wake: None }, "nothing to draw, no wake");
        pace.drawn(start, false);
        assert_eq!(pace.next(start + 1, Some(ANIMATION), CHANGE, 0), Next { draw: false, wake: Some(start + ANIMATION) });
        pace.changed(0);
        pace.drawn(start + 2, false);
        assert_eq!(pace.next(start + 3, None, CHANGE, 0), Next { draw: false, wake: None }, "a change drawn is not drawn again");
    }

    /// REQ-6400: while a channel is typed into, a change that is not the
    /// awaited page's waits for the page's frame and is drawn with it; with
    /// no such frame it is drawn when its wait is over.
    #[test]
    fn a_change_waits_for_the_awaited_frame_while_a_channel_is_typed_into() {
        const HOLD: u64 = 16;
        let mut pace = Pace::default();
        let key = 1_000;
        // The strip's figure changes as the key goes, long after the last frame.
        pace.changed(key);
        assert_eq!(pace.next(key, None, CHANGE, HOLD), Next { draw: false, wake: Some(key + HOLD) }, "no frame of its own at once");
        assert!(pace.next(key, None, CHANGE, 0).draw, "at once when no channel is typed into");
        // The page's answer comes three milliseconds later: one frame shows both.
        pace.awaited();
        assert!(pace.next(key + 3, None, CHANGE, HOLD).draw);
        pace.drawn(key + 3, true);
        assert_eq!(pace.next(key + 4, None, CHANGE, HOLD), Next { draw: false, wake: None }, "nothing left to draw");
        // With no answer, the change is drawn when its wait is over.
        pace.changed(2_000);
        pace.changed(2_010);
        assert!(!pace.next(2_015, None, CHANGE, HOLD).draw);
        assert!(pace.next(2_016, None, CHANGE, HOLD).draw, "counted from the first change waiting");
    }
}
