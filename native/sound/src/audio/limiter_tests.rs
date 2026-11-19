//! The limiter's tests (REQ-6933, TODO-6940): the ceiling, the bit-equal
//! pass below it, the fall of the gain before a peak, the release, the one
//! gain of a frame's channels, and its cost.

use super::*;

const RATE: u32 = 48_000;

/// A second of a sine of `hertz` and `peak` at [`RATE`].
fn sine(hertz: f64, peak: f64) -> Vec<i32> {
    #[expect(clippy::cast_possible_truncation, reason = "the peaks the tests ask for, rounded, are far within 32 bits")]
    (0..RATE).map(|at| (peak * (f64::from(at) * hertz * std::f64::consts::TAU / f64::from(RATE)).sin()).round() as i32).collect()
}

fn limited(limiter: &mut Limiter, input: &[i32]) -> Vec<i32> {
    let mut frames = input.to_vec();
    limiter.process(&mut frames);
    frames
}

fn loudest(frames: &[i32]) -> u32 {
    frames.iter().map(|sample| sample.unsigned_abs()).max().unwrap()
}

/// REQ-6933 item 2: the ceiling is -1 dBFS, rounded down.
#[test]
fn the_ceiling_is_one_decibel_under_full_scale() {
    #[expect(clippy::cast_possible_truncation, reason = "a fraction of a 16-bit full scale")]
    let worked_out = (f64::from(FULL_SCALE) * 10_f64.powf(-1.0 / 20.0)).floor() as i32;
    assert_eq!(CEILING, worked_out);
}

/// REQ-6933 items 1 and 2: a sine at +6 dBFS, which a clamp cuts flat for
/// two thirds of each cycle, comes out with every peak at or under the
/// ceiling, none at full scale, and still a sine at its tops.
#[test]
fn a_sine_six_decibels_over_full_scale_comes_out_at_the_ceiling() {
    let mut limiter = Limiter::new(RATE, 1);
    let output = limited(&mut limiter, &sine(1_000.0, 2.0 * f64::from(FULL_SCALE)));
    assert!(loudest(&output) <= CEILING.unsigned_abs(), "the loudest sample is {}", loudest(&output));
    assert!(output.iter().all(|sample| sample.abs() < FULL_SCALE), "a sample is at full scale");
    // Settled: turned down to the ceiling and no further, and each top one
    // sample long, as a 1 kHz sine's is at 48 kHz, where the clamp's was 16.
    let settled = &output[output.len() / 2..];
    let top = loudest(settled);
    assert!(top >= CEILING.unsigned_abs() - 300, "turned down too far: {top}");
    let mut run = 0;
    for sample in settled {
        run = if sample.unsigned_abs() == top { run + 1 } else { 0 };
        assert!(run <= 2, "the wave's top is flat");
    }
}

/// REQ-6933 item 5: a sine at -10 dBFS comes out bit for bit, the
/// look-ahead later; and again after the limiter has worked and let go.
#[test]
fn a_quiet_signal_passes_unchanged_after_the_delay() {
    let mut limiter = Limiter::new(RATE, 1);
    let delay = limiter.lookahead();
    assert_eq!(delay, 72, "1.5 ms at 48 kHz");
    let quiet = sine(1_000.0, f64::from(FULL_SCALE) * 10_f64.powf(-10.0 / 20.0));
    let output = limited(&mut limiter, &quiet);
    assert!(output[..delay].iter().all(|&sample| sample == 0), "silence until the first frame is out");
    assert_eq!(output[delay..], quiet[..quiet.len() - delay]);
    // Up to the ceiling itself nothing is turned down.
    let at_the_ceiling = sine(1_000.0, f64::from(CEILING));
    let mut limiter = Limiter::new(RATE, 1);
    assert_eq!(limited(&mut limiter, &at_the_ceiling)[delay..], at_the_ceiling[..at_the_ceiling.len() - delay]);
    // After a loud second and two quiet ones for the release, the gain is
    // exactly one again, not a step short of it.
    limited(&mut limiter, &sine(1_000.0, 4.0 * f64::from(FULL_SCALE)));
    limited(&mut limiter, &quiet);
    limited(&mut limiter, &quiet);
    let output = limited(&mut limiter, &quiet);
    assert_eq!(output[..delay], quiet[quiet.len() - delay..], "the end of the second before");
    assert_eq!(output[delay..], quiet[..quiet.len() - delay]);
}

/// REQ-6933 item 3, the look-ahead shown: with a steady level playing, one
/// sample four times over the ceiling turns the gain down before it goes
/// out, as a ramp over the look-ahead, and it goes out at the ceiling.
/// Prints the steepest step of the ramp, which [`LOOKAHEAD_US`] cites.
#[test]
fn the_gain_falls_as_a_ramp_before_a_peak() {
    const LEVEL: i32 = 10_000;
    const CLICK_AT: usize = 1_000;
    let mut limiter = Limiter::new(RATE, 1);
    let delay = limiter.lookahead();
    let mut input = vec![LEVEL; 4_000];
    input[CLICK_AT] = 4 * CEILING;
    let output = limited(&mut limiter, &input);
    let out_at = CLICK_AT + delay;
    assert_eq!(output[out_at - delay - 2], LEVEL, "untouched until the peak is seen");
    assert!((CEILING - 1..=CEILING).contains(&output[out_at]), "the peak goes out at the ceiling: {}", output[out_at]);
    let ramp = &output[out_at - delay - 1..out_at];
    assert!(ramp.windows(2).all(|pair| pair[1] < pair[0]), "the level falls at every frame of the look-ahead: {ramp:?}");
    assert!(ramp[delay] <= LEVEL / 4 + LEVEL / 50, "and has all but reached the peak's gain, a quarter, the frame before it: {}", ramp[delay]);
    let steepest = ramp.windows(2).map(|pair| pair[0] - pair[1]).max().unwrap();
    println!(
        "limiter: the gain falls by a quarter over {} frames, at most {:.2} % of unity a frame",
        delay + 1,
        f64::from(steepest) * 100.0 / f64::from(LEVEL)
    );
    assert!(steepest <= LEVEL / 50, "no step of the ramp is more than 2 % of unity: {steepest}");
    // Then it comes back, never above the level that came in.
    assert!(output[out_at + 1..].iter().all(|&sample| sample <= LEVEL));
    assert!(output[out_at + 1..].windows(2).all(|pair| pair[1] >= pair[0]), "the release only rises");
}

/// The gains for a second of `input`, by the limiter's own steps.
fn gains(limiter: &mut Limiter, input: &[i32]) -> Vec<u32> {
    input
        .iter()
        .map(|sample| {
            let gain = limiter.gain_after(sample.unsigned_abs());
            limiter.at = limiter.at.wrapping_add(1).checked_rem(limiter.window).unwrap();
            gain
        })
        .collect()
}

/// REQ-6933 item 3, the release: a 100 Hz sine held at +6 dBFS moves the
/// settled gain by a few per cent over its cycle, so the wave keeps its
/// shape, and after one click the gain is back near one within half a
/// second. Prints the figures [`RELEASE_MS`] cites, for 100 milliseconds
/// and for 50.
#[test]
fn a_low_note_held_over_the_ceiling_barely_moves_the_gain() {
    let low = sine(100.0, 2.0 * f64::from(FULL_SCALE));
    let ripple = |release_ms: u32| {
        let mut limiter = Limiter::new(RATE, 1);
        limiter.release_frames = RATE / 1_000 * release_ms;
        let settled = gains(&mut limiter, &low)[low.len() / 2..].to_vec();
        let (least, most) = (*settled.iter().min().unwrap(), *settled.iter().max().unwrap());
        f64::from(most - least) * 100.0 / f64::from(most)
    };
    let (chosen, shorter) = (ripple(100), ripple(50));
    println!("limiter: a 100 Hz sine at +6 dBFS moves the settled gain by {chosen:.1} % with a 100 ms release, {shorter:.1} % with 50 ms");
    assert_eq!(Limiter::new(RATE, 1).release_frames, RATE / 10, "100 ms is the release built");
    assert!(chosen < 3.0, "{chosen}");
    assert!(shorter > chosen);
    // One click four times over the ceiling, then silence.
    let mut limiter = Limiter::new(RATE, 1);
    let mut click = vec![0; low.len()];
    click[0] = 4 * CEILING;
    let after = gains(&mut limiter, &click);
    let back = after.iter().rposition(|&gain| gain < UNITY / 100 * 99).unwrap();
    println!("limiter: after a click four times over the ceiling the gain is within 1 % of one after {} ms", back * 1_000 / usize::try_from(RATE).unwrap());
    assert!(back < usize::try_from(RATE).unwrap() / 2, "{back}");
    assert_eq!(*after.last().unwrap(), UNITY, "and arrives at one exactly");
}

/// REQ-6933 item 1: whatever comes in, the 32-bit extremes and
/// `i32::MIN` among it, nothing comes out over the ceiling, so the last
/// guard, the clamp of [`narrowed`], is never reached.
#[test]
fn nothing_that_comes_in_reaches_the_last_guard() {
    let mut limiter = Limiter::new(RATE, 1);
    // A linear congruential generator (Knuth's MMIX constants): the same
    // noise every run.
    let mut state = 1_u64;
    let mut noise = || {
        state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        u32::try_from(state >> 32).unwrap().cast_signed()
    };
    let mut input: Vec<i32> = (0..200_000).map(|at| if at % 3 == 0 { noise() } else { noise() >> (at % 29) }).collect();
    input[5_000] = i32::MIN;
    input[5_001] = i32::MAX;
    input[100_000..100_100].fill(i32::MIN);
    let output = limited(&mut limiter, &input);
    assert!(loudest(&output) <= CEILING.unsigned_abs(), "{}", loudest(&output));
    assert!(output.iter().all(|&sample| i32::from(narrowed(sample)) == sample), "the guard changed a sample");
}

/// Kept ready for stereo (DEC-6924 item 10 (ii)): the channels of a frame
/// are turned down by one gain, that of the loudest, so the quieter
/// channel keeps its level against the louder and the image does not move.
#[test]
fn the_channels_of_a_frame_share_one_gain() {
    let mut limiter = Limiter::new(RATE, 2);
    let (left, right) = (2 * CEILING, CEILING / 2);
    let mut frames: Vec<i32> = [left, right].repeat(4_000);
    limiter.process(&mut frames);
    let settled = &frames[frames.len() - 2..];
    assert!((CEILING - 1..=CEILING).contains(&settled[0]), "the loud channel at the ceiling: {}", settled[0]);
    assert!((CEILING / 4 - 1..=CEILING / 4).contains(&settled[1]), "the quiet one a quarter of it, as it came in: {}", settled[1]);
    // And each channel is delayed as itself: the first frames out are the
    // silence before them, then the first frame in.
    let delay = limiter.lookahead();
    assert!(frames[..2 * delay].iter().all(|&sample| sample == 0));
    assert!(frames[2 * delay] > frames[2 * delay + 1] && frames[2 * delay + 1] > 0);
}

/// The end of a sound is still in the delay when its last frame came in:
/// the limiter says so until it has gone out.
#[test]
fn it_holds_sound_until_the_last_frame_is_out() {
    let mut limiter = Limiter::new(RATE, 1);
    let delay = limiter.lookahead();
    assert!(!limiter.holds_sound());
    limited(&mut limiter, &[1_000; 10]);
    assert!(limiter.holds_sound());
    let tail = limited(&mut limiter, &vec![0; delay]);
    assert_eq!(tail[delay - 10..], [1_000; 10], "the sound went out in these frames");
    assert!(!limiter.holds_sound(), "and nothing is left");
    assert!(limited(&mut limiter, &vec![0; 2 * delay]).iter().all(|&sample| sample == 0));
}

/// A device may name any rate: the buffers are sized for at most
/// [`MAX_RATE_HZ`], and for at least one frame of look-ahead.
#[test]
fn the_buffers_are_bounded_whatever_rate_the_device_names() {
    let limiter = Limiter::new(u32::MAX, 2);
    assert_eq!(limiter.lookahead(), 1_152, "1.5 ms at 768 kHz");
    assert_eq!(limiter.delay.len(), 2 * 1_153);
    assert_eq!(Limiter::new(0, 0).lookahead(), 1);
}

/// REQ-6933 item 4: the limiter's own cost for a device buffer of 10
/// milliseconds, 480 frames, printed, and held under a twentieth of the
/// buffer's time in a build of any kind: a debug build, which checks
/// every index and sum, took 109 microseconds on 20 November 2026, and a
/// release build 11.
#[test]
fn a_device_buffer_costs_the_limiter_under_a_twentieth_of_its_time() {
    const BLOCK: usize = 480;
    const BLOCKS: usize = 2_000;
    const BUDGET: std::time::Duration = std::time::Duration::from_micros(500);
    for (name, peak) in [("over the ceiling", 12.0 * f64::from(FULL_SCALE)), ("under the ceiling", 2_000.0)] {
        let mut limiter = Limiter::new(RATE, 1);
        let wave = sine(440.0, peak);
        let mut costs = Vec::with_capacity(BLOCKS);
        for block in 0..BLOCKS {
            let start = block * BLOCK % (wave.len() - BLOCK);
            let mut frames = wave[start..start + BLOCK].to_vec();
            let started = std::time::Instant::now();
            limiter.process(&mut frames);
            costs.push(started.elapsed());
            std::hint::black_box(&frames);
        }
        costs.sort();
        println!("limiter: a 10 ms buffer {name}: median {:?}, slowest {:?}", costs[BLOCKS / 2], costs[BLOCKS - 1]);
        assert!(costs[BLOCKS / 2] < BUDGET, "{name}: the median, {:?}, is over {BUDGET:?}", costs[BLOCKS / 2]);
    }
}
