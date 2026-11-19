//! The look-ahead limiter on the helper's final output (REQ-6933, DEC-6924
//! item 10 (i), DEC-6950): the sum of every tab's mixer, which may lie far
//! above full scale, is turned down smoothly before a peak arrives and let
//! back up slowly after it, in place of a clamp that cut the wave's tops.
//!
//! How it works, for each frame:
//! 1. the gain the frame needs to come out at the ceiling is worked out
//!    from its loudest channel (one gain for all channels, so a stereo
//!    image would not shift: only one channel is fed today);
//! 2. that gain may rise again only at the release's pace, and falls at
//!    once;
//! 3. the least such gain of the last `lookahead + 1` frames is taken, and
//! 4. the mean of the last `lookahead + 1` of those is the gain applied to
//!    the frame that came in `lookahead` frames ago.
//!
//! Step 3 holds the gain down from the moment a peak comes in until it
//! goes out, and step 4 turns the fall into a ramp that ends as the peak
//! goes out. Every gain averaged in step 4 is a least value over a window
//! that holds the outgoing frame, so their mean is never above the gain
//! that frame needs: the output never passes the ceiling, by construction
//! and not by a clamp. A frame that needs no turning down while none near
//! it does is multiplied by exactly one, and comes out bit for bit.
//!
//! All of it is integer arithmetic on buffers made once, in
//! [`Limiter::new`], before the audio thread runs: [`Limiter::process`]
//! never allocates, frees, locks or calls the operating system.

/// Full scale: the largest magnitude a 16-bit sample holds on both sides
/// of zero.
const FULL_SCALE: i32 = 32_767;
const _: () = assert!(FULL_SCALE == i16::MAX as i32);

/// The ceiling no output sample passes: -1 dBFS (decibels relative to full
/// scale; REQ-6933 item 2), which leaves room for a converter that
/// overshoots between samples. `10^(-1/20) = 0.891 250 9`, and
/// `32 767 × 0.891 250 9 = 29 203.6`, rounded down so the ceiling is never
/// above -1 dBFS. A test works it out again in floating point.
pub const CEILING: i32 = 29_203;
const _: () = assert!(CEILING < FULL_SCALE);

/// The bits of fraction a gain holds: gains are fixed-point numbers of
/// [`UNITY`] to one. 24 bits keep a gain exact enough for a sum of four
/// thousand full-scale voices (2^27) to come out within a thousandth of
/// the ceiling, and `2^31 × 2^24` still fits the 64 bits the product is
/// made in.
const GAIN_BITS: u32 = 24;

/// The gain of one: the sample goes out as it came in.
const UNITY: u32 = 1 << GAIN_BITS;

/// How far ahead the limiter looks, in microseconds, which is also how
/// long it delays everything it passes. 1.5 milliseconds (DEC-6950 item
/// 4), not the 5 that REQ-6933 proposed, because the delay is added to
/// every note of a page's local sounds, whose whole budget from key to
/// sound is 12 milliseconds and 5 in the median (page-rules.md RULE-64).
/// Measured at 48 kHz (20 November 2026, the test
/// `the_gain_falls_as_a_ramp_before_a_peak`): the delay is 72 frames, and
/// before a click four times over the ceiling the gain falls over 73
/// frames by at most 1.03 % of unity from one frame to the next, a ramp
/// and not a step.
const LOOKAHEAD_US: u64 = 1_500;

/// The release's time constant, in milliseconds: after a peak has gone
/// out, the gain covers 63 % of what is left of its way back to one in
/// each such time. Measured at 48 kHz with a 100 Hz sine at +6 dBFS (20
/// November 2026, the test
/// `a_low_note_held_over_the_ceiling_barely_moves_the_gain`): the gain,
/// once settled, moves by 2.8 % of itself over the note's cycle with 100
/// milliseconds, against 5.1 % with 50, so the wave keeps its shape; and
/// after a single click four times over the ceiling the gain is back
/// within 1 % of one in 435 milliseconds.
const RELEASE_MS: u64 = 100;

/// The highest output rate the limiter sizes its buffers for: four times
/// 192 kHz, above any rate an output device offers. A device that names a
/// higher one is looked ahead by this many hertz' worth of frames, so the
/// device's word never sizes an allocation without a bound.
const MAX_RATE_HZ: u32 = 768_000;

const MICROSECONDS_PER_SECOND: u64 = 1_000_000;
const MILLISECONDS_PER_SECOND: u64 = 1_000;

/// One gain waiting in the window of step 3, and the frame it came in at.
#[derive(Clone, Copy)]
struct Waiting {
    gain: u32,
    frame: u32,
}

/// The limiter and everything it remembers between buffers.
pub struct Limiter {
    /// The channels of a frame. They share one gain.
    channels: usize,
    /// The frames it looks ahead by and delays by.
    lookahead: usize,
    /// The frames of step 3's and step 4's windows: `lookahead + 1`.
    window: usize,
    /// The frames the release's time constant lasts.
    release_frames: u32,
    /// The last `window` frames that came in, interleaved, as a ring.
    delay: Box<[i32]>,
    /// The last `window` gains of step 3, as a ring, and their sum.
    least: Box<[u32]>,
    least_sum: u64,
    /// Where in both rings the next frame goes.
    at: usize,
    /// Step 2's gain for the last frame.
    released: u32,
    /// Step 3's window: the gains that may yet be the least, rising from
    /// `head`, as a ring that holds at most `window`.
    waiting: Box<[Waiting]>,
    head: usize,
    held: usize,
    /// The frames that have come in, wrapping.
    frame: u32,
    /// The frames until the delay holds only silence.
    tail: usize,
}

impl Limiter {
    /// A limiter for frames of `channels` samples at `rate_hz`, passing
    /// everything unchanged until something loud comes in. Every buffer it
    /// will ever use is made here.
    #[must_use]
    pub fn new(rate_hz: u32, channels: usize) -> Self {
        let rate = u64::from(rate_hz.min(MAX_RATE_HZ));
        let frames = |per_second: u64, time: u64| rate.saturating_mul(time).checked_div(per_second).unwrap_or(0).max(1);
        let lookahead = usize::try_from(frames(MICROSECONDS_PER_SECOND, LOOKAHEAD_US)).unwrap_or(1);
        let window = lookahead.saturating_add(1);
        let channels = channels.max(1);
        Self {
            channels,
            lookahead,
            window,
            release_frames: u32::try_from(frames(MILLISECONDS_PER_SECOND, RELEASE_MS)).unwrap_or(u32::MAX),
            delay: vec![0; window.saturating_mul(channels)].into_boxed_slice(),
            least: vec![UNITY; window].into_boxed_slice(),
            least_sum: u64::from(UNITY).saturating_mul(u64::try_from(window).unwrap_or(u64::MAX)),
            at: 0,
            released: UNITY,
            waiting: vec![Waiting { gain: UNITY, frame: 0 }; window].into_boxed_slice(),
            head: 0,
            held: 0,
            frame: 0,
            tail: 0,
        }
    }

    /// The frames everything it passes is delayed by.
    #[must_use]
    pub const fn lookahead(&self) -> usize {
        self.lookahead
    }

    /// Whether sound that came in has yet to go out: the device must run
    /// on until it has, or the end of a sound would wait in the delay and
    /// be heard at the start of the next.
    #[must_use]
    pub const fn holds_sound(&self) -> bool {
        self.tail > 0
    }

    /// Limits `frames`, interleaved frames of its channels, in place: each
    /// comes out as the frame that came in [`Limiter::lookahead`] frames
    /// before, turned down as far as the ceiling needs. No sample that
    /// comes out is above [`CEILING`] in magnitude. It never allocates,
    /// frees or locks.
    pub fn process(&mut self, frames: &mut [i32]) {
        for frame in frames.chunks_exact_mut(self.channels) {
            let peak = frame.iter().fold(0, |peak, sample| peak.max(sample.unsigned_abs()));
            self.tail = if peak == 0 { self.tail.saturating_sub(1) } else { self.lookahead };
            let gain = self.gain_after(peak);
            let next = self.at.saturating_add(1).checked_rem(self.window).unwrap_or(0);
            // The ring holds one frame more than the delay: the oldest
            // frame, which goes out now, is the one after the newest.
            for (channel, sample) in frame.iter_mut().enumerate() {
                let newest = self.at.saturating_mul(self.channels).saturating_add(channel);
                let oldest = next.saturating_mul(self.channels).saturating_add(channel);
                if let Some(slot) = self.delay.get_mut(newest) {
                    *slot = *sample;
                }
                *sample = scaled(self.delay.get(oldest).copied().unwrap_or(0), gain);
            }
            self.at = next;
        }
    }

    /// Steps 1 to 4 for a frame whose loudest sample is `peak`: the gain
    /// for the frame that goes out now.
    fn gain_after(&mut self, peak: u32) -> u32 {
        // Step 1: the gain that brings this frame to the ceiling.
        let ceiling = CEILING.unsigned_abs();
        let needed = if peak <= ceiling {
            UNITY
        } else {
            let exact = u64::from(ceiling).saturating_mul(u64::from(UNITY)).checked_div(u64::from(peak)).unwrap_or(0);
            u32::try_from(exact).unwrap_or(UNITY)
        };
        // Step 2: back towards one at the release's pace, by at least one
        // step so that it arrives; down at once.
        let step = UNITY.saturating_sub(self.released).checked_div(self.release_frames).unwrap_or(0).max(1);
        self.released = needed.min(self.released.saturating_add(step).min(UNITY));
        // Step 3: the least of the last `window` such gains. A gain
        // waiting that is no less than the new one can never be the least
        // again; one that came in a window ago has gone out.
        let window = self.window;
        let place = |from_head: usize, head: usize| head.saturating_add(from_head).checked_rem(window).unwrap_or(0);
        let age_out = u32::try_from(window).unwrap_or(u32::MAX);
        while self.held > 0 && self.waiting.get(self.head).is_some_and(|first| self.frame.wrapping_sub(first.frame) >= age_out) {
            self.head = place(1, self.head);
            self.held = self.held.saturating_sub(1);
        }
        while self.held > 0 && self.waiting.get(place(self.held.saturating_sub(1), self.head)).is_some_and(|last| last.gain >= self.released) {
            self.held = self.held.saturating_sub(1);
        }
        if let Some(slot) = self.waiting.get_mut(place(self.held, self.head)) {
            *slot = Waiting { gain: self.released, frame: self.frame };
            self.held = self.held.saturating_add(1);
        }
        self.frame = self.frame.wrapping_add(1);
        let least = self.waiting.get(self.head).map_or(self.released, |first| first.gain);
        // Step 4: the mean of the last `window` of those.
        if let Some(slot) = self.least.get_mut(self.at) {
            self.least_sum = self.least_sum.saturating_sub(u64::from(*slot)).saturating_add(u64::from(least));
            *slot = least;
        }
        let mean = self.least_sum.checked_div(u64::try_from(window).unwrap_or(u64::MAX)).unwrap_or(0);
        u32::try_from(mean).unwrap_or(UNITY).min(UNITY)
    }
}

/// `sample` at `gain`, rounded towards zero, so it is never louder than
/// the gain allows. At a gain of one it is the sample itself.
fn scaled(sample: i32, gain: u32) -> i32 {
    let product = i64::from(sample).saturating_mul(i64::from(gain));
    i32::try_from(product.checked_div(i64::from(UNITY)).unwrap_or(0)).unwrap_or(0)
}

/// The 16-bit sample of one the limiter passed. The clamp is a last guard
/// (REQ-6933 item 1): the limiter's tests show nothing reaches it.
#[must_use]
pub fn narrowed(sample: i32) -> i16 {
    i16::try_from(sample.clamp(i32::from(i16::MIN), i32::from(i16::MAX))).unwrap_or(0)
}

#[cfg(test)]
#[path = "limiter_tests.rs"]
mod tests;
