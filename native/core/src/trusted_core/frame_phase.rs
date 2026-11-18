//! The interface's frames, passed on to the decoders (neoteletext-common's
//! `docs/page-rules.md` RULE-98 item 2): bounds checked, otherwise unread,
//! and told to each decoder relative to the moment it is written, since
//! each process reads the monotonic clock only for itself (DEC-6659 item
//! 12).

use protocol::decoder::ToDecoder;
use protocol::rules::{FramePhase, PHASE_STALE_MILLISECONDS};

use super::Core;

/// Microseconds in a millisecond.
const MICROSECONDS_PER_MILLISECOND: u64 = 1_000;

/// The interface's compositions, on the trusted core's clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Compositions {
    /// The display's refresh period, in microseconds.
    period: u32,
    /// One composition, in microseconds since the trusted core started.
    at: u64,
    /// When the interface told it.
    told: u64,
}

impl Compositions {
    /// What `phase`, read at `now_us`, says.
    pub(super) const fn of(phase: FramePhase, now_us: u64) -> Self {
        Self { period: phase.period_us, at: phase.composition_after(now_us), told: now_us }
    }

    /// The phase to tell a decoder at `now_us`: none once the interface's
    /// word is older than a decoder would keep it (RULE-98 item 4), so a
    /// decoder started later is not given a stale phase as a fresh one.
    pub(super) fn phase_at(self, now_us: u64) -> Option<FramePhase> {
        let stale_us = PHASE_STALE_MILLISECONDS.saturating_mul(MICROSECONDS_PER_MILLISECOND);
        if now_us.saturating_sub(self.told) >= stale_us {
            return None;
        }
        FramePhase::at(self.period, self.at, now_us)
    }
}

impl Core {
    /// Microseconds since the trusted core started, the clock the
    /// interface's compositions are kept on.
    fn phase_clock_us(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_micros()).unwrap_or(u64::MAX)
    }

    /// The interface told its frames: kept, and passed to every decoder,
    /// each of which paints its pages on screen just before a composition
    /// (RULE-65) and locks its open page's ticks to them (RULE-98).
    pub(super) fn frame_phase(&mut self, phase: FramePhase) {
        self.frame_phase = Some(Compositions::of(phase, self.phase_clock_us()));
        let channels: Vec<u32> = self.decoders.keys().copied().collect();
        for channel in channels {
            self.tell_frame_phase(channel);
        }
    }

    /// Tells `channel`'s decoder the interface's frames as they stand now,
    /// as a decoder starts or a page opens in its view (RULE-98 item 2).
    /// Never kept for a sleeping decoder: a phase is true only as it is
    /// written.
    pub(super) fn tell_frame_phase(&mut self, channel: u32) {
        let now_us = self.phase_clock_us();
        let Some(phase) = self.frame_phase.and_then(|compositions| compositions.phase_at(now_us)) else { return };
        if let Some(decoder) = self.decoders.get_mut(&channel) {
            let _ = decoder.tell(&ToDecoder::FramePhase(phase));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RULE-98 item 2: the phase told later is the same compositions,
    /// relative to the later moment, and none once stale.
    #[test]
    fn a_phase_is_passed_on_relative_to_its_writing() {
        let kept = Compositions::of(FramePhase { period_us: 16_667, next_in_us: 4_000 }, 100_000);
        assert_eq!(kept.phase_at(100_000), Some(FramePhase { period_us: 16_667, next_in_us: 4_000 }));
        assert_eq!(kept.phase_at(103_000), Some(FramePhase { period_us: 16_667, next_in_us: 1_000 }));
        assert_eq!(kept.phase_at(105_000), Some(FramePhase { period_us: 16_667, next_in_us: 15_667 }));
        assert_eq!(kept.phase_at(100_000 + 999_999).map(|phase| phase.period_us), Some(16_667));
        assert_eq!(kept.phase_at(1_100_000), None, "a phase a second old is stale");
    }
}
