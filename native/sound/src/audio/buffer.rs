//! The size of the output device's buffer (neoteletext-common's
//! `docs/page-rules.md` RULE-97, the owner's decision DEC-6580 item 2): the
//! usual 10 milliseconds, and a small one while a local sound is open, so a
//! key's note waits for half as long; given up for the usual size at the
//! first underrun or overrun while it is in use, and kept a little after
//! the last local sound ends, so that going from one playing page to
//! another does not reopen the device twice.
//!
//! This is the decision alone, with no device: the helper's loop tells it
//! what happened and opens the device at the size it says.

use std::time::{Duration, Instant};

/// The device buffer the helper opens with, in frames at
/// [`REFERENCE_RATE_HZ`]: 480, 10 milliseconds (RULE-97).
pub const USUAL_DEVICE_FRAMES: u32 = 480;
/// The device buffer asked for while a local sound is open, in frames at
/// [`REFERENCE_RATE_HZ`]: 256, 5.33 milliseconds (RULE-97).
pub const LOCAL_DEVICE_FRAMES: u32 = 256;
/// The least device buffer taken where the device offers a range: 128
/// frames at [`REFERENCE_RATE_HZ`], 2.67 milliseconds (RULE-97 item 1).
pub const LEAST_DEVICE_FRAMES: u32 = 128;
/// How long the small buffer is kept after the last local sound ends
/// (RULE-97 item 4).
pub const LOCAL_DEVICE_LINGER_MILLISECONDS: u64 = 2_000;
/// The rate the frame counts above are given at: at another device rate
/// the number of frames nearest the same time is used (RULE-97).
pub const REFERENCE_RATE_HZ: u32 = 48_000;

/// What the device offers for its buffer's size, in frames at its own rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offered {
    /// Any size from the first to the second.
    Range(u32, u32),
    /// Only its own size.
    Own,
}

/// The size the device is to be opened with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// The device's own size, as it was opened before the small buffer.
    Own,
    /// This many frames, at the device's rate.
    Frames(u32),
}

/// `frames` at [`REFERENCE_RATE_HZ`] as the nearest number of frames at
/// `rate_hz`.
#[must_use]
pub fn at_rate(frames: u32, rate_hz: u32) -> u32 {
    let scaled = u64::from(frames).saturating_mul(u64::from(rate_hz)).saturating_add(u64::from(REFERENCE_RATE_HZ / 2));
    u32::try_from(scaled.checked_div(u64::from(REFERENCE_RATE_HZ)).unwrap_or(0)).unwrap_or(u32::MAX)
}

/// The size to open the device with for the usual buffer.
#[must_use]
pub fn usual(offered: Offered, rate_hz: u32) -> Size {
    match offered {
        Offered::Range(least, most) => Size::Frames(at_rate(USUAL_DEVICE_FRAMES, rate_hz).clamp(least, most.max(least))),
        Offered::Own => Size::Own,
    }
}

/// The size to open the device with for the small buffer (RULE-97 item
/// 1): the size in the range nearest [`LOCAL_DEVICE_FRAMES`] and not below
/// [`LEAST_DEVICE_FRAMES`]; none where the device grants only its own size,
/// or where the least it grants is more than the usual buffer.
#[must_use]
pub fn small(offered: Offered, rate_hz: u32) -> Option<u32> {
    let Offered::Range(least, most) = offered else { return None };
    let wanted = at_rate(LOCAL_DEVICE_FRAMES, rate_hz).max(at_rate(LEAST_DEVICE_FRAMES, rate_hz));
    let floor = least.max(at_rate(LEAST_DEVICE_FRAMES, rate_hz));
    let granted = wanted.clamp(floor, most.max(floor));
    (granted <= at_rate(USUAL_DEVICE_FRAMES, rate_hz) && granted <= most).then_some(granted)
}

/// Where the policy stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// The usual buffer: no local sound is open, or the device grants no
    /// smaller.
    Usual,
    /// The small buffer, while a local sound is open.
    Small,
    /// The small buffer kept after the last local sound ended, until then.
    Lingering(Instant),
    /// Given up for the usual size at an underrun or overrun, until every
    /// local sound open then has ended (RULE-97 item 3).
    FallenBack,
}

/// What the helper's loop is to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Nothing.
    None,
    /// Open the device again with the small buffer.
    Small,
    /// Open the device again with the usual buffer.
    Usual,
    /// Open the device again with the usual buffer, and report fault 7
    /// with count 0 once to each view with a local sound (RULE-97 item 3).
    FallBack,
}

/// The policy of RULE-97.
#[derive(Debug)]
pub struct Policy {
    state: State,
    /// The device grants a small buffer at all.
    can: bool,
}

impl Policy {
    /// The policy for a device that can (`can`) or cannot be given a small
    /// buffer.
    #[must_use]
    pub const fn new(can: bool) -> Self {
        Self { state: State::Usual, can }
    }

    /// Whether the small buffer is in use.
    #[must_use]
    pub const fn is_small(&self) -> bool {
        matches!(self.state, State::Small | State::Lingering(_))
    }

    /// The number of local sounds open in every view changed to `open`.
    pub fn locals(&mut self, open: usize, now: Instant) -> Change {
        match (self.state, open) {
            (State::Usual, 1..) if self.can => {
                self.state = State::Small;
                Change::Small
            }
            // A local sound within the linger: no change of size.
            (State::Lingering(_), 1..) => {
                self.state = State::Small;
                Change::None
            }
            (State::Small, 0) => {
                self.state = State::Lingering(now.checked_add(Duration::from_millis(LOCAL_DEVICE_LINGER_MILLISECONDS)).unwrap_or(now));
                Change::None
            }
            // Every local sound open at the fallback has ended: the next
            // asks for the small buffer again.
            (State::FallenBack, 0) => {
                self.state = State::Usual;
                Change::None
            }
            _ => Change::None,
        }
    }

    /// The device reported an underrun, or a buffer took longer to fill
    /// than its own time.
    pub fn trouble(&mut self) -> Change {
        if self.is_small() {
            self.state = State::FallenBack;
            Change::FallBack
        } else {
            Change::None
        }
    }

    /// When the loop is next to call [`Policy::wake`]: the linger's end.
    #[must_use]
    pub const fn due(&self) -> Option<Instant> {
        match self.state {
            State::Lingering(until) => Some(until),
            _ => None,
        }
    }

    /// Time passed: a linger that ended goes back to the usual buffer.
    pub fn wake(&mut self, now: Instant) -> Change {
        match self.state {
            State::Lingering(until) if now >= until => {
                self.state = State::Usual;
                Change::Usual
            }
            _ => Change::None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    const RANGE: Offered = Offered::Range(64, 4096);

    /// RULE-97 item 1 (REQ-6535 item 9): the small buffer asked for is 256
    /// frames at 48 kHz, the same time at another rate, never below 128;
    /// a device that grants only its own size, or none at or below the
    /// usual 480, is left as it was.
    #[test]
    fn the_small_buffer_is_the_nearest_the_device_grants() {
        assert_eq!(small(RANGE, 48_000), Some(256));
        assert_eq!(small(RANGE, 44_100), Some(235), "the same 5.33 ms");
        assert_eq!(small(Offered::Range(200, 4096), 48_000), Some(256));
        assert_eq!(small(Offered::Range(300, 4096), 48_000), Some(300), "the nearest in the range");
        assert_eq!(small(Offered::Range(16, 100), 48_000), None, "nothing at or above 128");
        assert_eq!(small(Offered::Range(600, 4096), 48_000), None, "a device that grants more than 480 is left alone");
        assert_eq!(small(Offered::Own, 48_000), None, "a device that grants only its own size");
        assert_eq!(usual(RANGE, 48_000), Size::Frames(480));
        assert_eq!(usual(Offered::Own, 48_000), Size::Own);
    }

    /// RULE-97 items 1, 3 and 4 (REQ-6535 item 9): the first local sound
    /// asks for the small buffer; a second within the linger changes
    /// nothing; the last's end keeps it for two seconds, then the usual;
    /// an underrun or overrun gives it up once, reported, until every
    /// local sound open then has ended; a page with no local sound never
    /// changes the size.
    #[test]
    fn the_buffer_follows_the_local_sounds() {
        let start = Instant::now();
        let mut policy = Policy::new(true);
        assert_eq!(policy.locals(0, start), Change::None, "no local sound, no change");
        assert_eq!(policy.locals(1, start), Change::Small);
        assert_eq!(policy.locals(2, start), Change::None);
        assert_eq!(policy.locals(0, start), Change::None, "the linger begins");
        let linger = Duration::from_millis(LOCAL_DEVICE_LINGER_MILLISECONDS);
        assert_eq!(policy.due(), Some(start + linger));
        assert_eq!(policy.wake(start + linger / 2), Change::None);
        assert_eq!(policy.locals(1, start + linger / 2), Change::None, "a local sound within the linger: no reopening");
        assert_eq!(policy.locals(0, start + linger), Change::None);
        assert_eq!(policy.wake(start + linger * 2), Change::Usual);
        assert!(!policy.is_small());

        assert_eq!(policy.locals(1, start), Change::Small);
        assert_eq!(policy.trouble(), Change::FallBack, "the first underrun gives the small buffer up");
        assert_eq!(policy.trouble(), Change::None, "and is reported once");
        assert_eq!(policy.locals(2, start), Change::None, "it stays usual while those sounds are open");
        assert_eq!(policy.locals(0, start), Change::None);
        assert_eq!(policy.locals(1, start), Change::Small, "a local sound after the fallback ended asks again");

        let mut never = Policy::new(false);
        assert_eq!(never.locals(1, start), Change::None, "a device that grants no smaller buffer");
        assert_eq!(never.trouble(), Change::None);
    }
}
