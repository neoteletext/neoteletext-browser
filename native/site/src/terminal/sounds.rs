//! The channel's sounds: the piano's notes and the effects of the game
//! and the typing page, synthesised, with the handles and channels they
//! take in the audio profile.

use crate::podcast;

/// The piano's keys: by position, by letter, and the note's frequency in
/// hertz (equal temperament from A4 = 440 Hz: C4 to C5).
pub(super) const PIANO: [(&str, char, f64); 8] = [
    ("KeyA", 'a', 261.63),
    ("KeyS", 's', 293.66),
    ("KeyD", 'd', 329.63),
    ("KeyF", 'f', 349.23),
    ("KeyG", 'g', 392.00),
    ("KeyH", 'h', 440.00),
    ("KeyJ", 'j', 493.88),
    ("KeyK", 'k', 523.25),
];
/// The notes' rate and length: half a second at 8 kHz, IMA ADPCM in
/// 256-byte blocks, the profile's smallest.
pub(super) const NOTE_RATE: u32 = 8_000;
const NOTE_SAMPLES: usize = 4_000;
pub(super) const NOTE_BLOCK: u16 = 256;
/// A note's peak, a quarter of full scale, and its decay.
const NOTE_PEAK: f64 = 8_000.0;
const NOTE_DECAY: f64 = 6.0;
/// The handles the notes take: 1 to 8.
pub(super) const FIRST_HANDLE: u8 = 1;
/// The game's sounds: a shot, a hit, a win and a loss, after the notes.
pub(super) const FIRE_HANDLE: u8 = 9;
pub(super) const HIT_HANDLE: u8 = 10;
pub(super) const WIN_HANDLE: u8 = 11;
pub(super) const LOSE_HANDLE: u8 = 12;
/// The typing page's sounds: a key's click, and Enter's bell.
pub(super) const CLICK_HANDLE: u8 = 13;
pub(super) const BELL_HANDLE: u8 = 14;
/// A click: 25 ms of noise falling away fast, as a typewriter's key.
const CLICK_SAMPLES: u32 = 200;
const CLICK_DECAY: f64 = 6.0;
/// A bell: E6, ringing for half a second.
const BELL_HZ: f64 = 1_318.51;
const BELL_SAMPLES: u32 = 4_000;
/// A shot sweeps down from this pitch to this one, in hertz.
const FIRE_FROM_HZ: f64 = 1_400.0;
const FIRE_TO_HZ: f64 = 300.0;
const FIRE_SAMPLES: u32 = 1_200;
/// A hit is noise, a quarter of a second.
const HIT_SAMPLES: u32 = 2_000;
/// A win rises C E G C; a loss falls G E C, each note this long.
const TUNE_NOTE_SAMPLES: u32 = 1_000;
const WIN_TUNE: [f64; 4] = [523.25, 659.25, 783.99, 1_046.50];
const LOSE_TUNE: [f64; 3] = [392.00, 329.63, 196.00];
/// The channels the effects take in turn: the first three of the audio
/// profile's four (REQ-AUD-010), the fourth being the podcast's.
pub(super) const AUDIO_CHANNELS: u8 = 3;
const _: () = assert!(podcast::CHANNEL == AUDIO_CHANNELS, "the podcast has a channel of its own");
pub(super) const NOTE_GAIN: u8 = 80;
pub(super) const NOTE_FADE_MS: u16 = 5;
/// The audio service the channel's sounds are on.
pub const AUDIO_SERVICE: u8 = 0;

/// A shot: a sine sweeping down, fading out.
#[allow(clippy::cast_possible_truncation, reason = "the peak is a quarter of the samples' range")]
fn fire_sound() -> Vec<i16> {
    let mut phase = 0.0_f64;
    (0..FIRE_SAMPLES)
        .map(|index| {
            let share = f64::from(index) / f64::from(FIRE_SAMPLES);
            let frequency = FIRE_FROM_HZ + (FIRE_TO_HZ - FIRE_FROM_HZ) * share;
            phase += 2.0 * std::f64::consts::PI * frequency / f64::from(NOTE_RATE);
            (phase.sin() * NOTE_PEAK * (1.0 - share)).round() as i16
        })
        .collect()
}

/// Noise from a linear congruential generator (Numerical Recipes'
/// constants), shaped by `envelope` over `samples`.
#[allow(clippy::cast_possible_truncation, reason = "the peak is a quarter of the samples' range")]
fn noise(samples: u32, envelope: impl Fn(f64) -> f64) -> Vec<i16> {
    const MULTIPLIER: u32 = 1_664_525;
    const INCREMENT: u32 = 1_013_904_223;
    let mut state = 1_u32;
    (0..samples)
        .map(|index| {
            state = state.wrapping_mul(MULTIPLIER).wrapping_add(INCREMENT);
            let noise = f64::from(state >> 16) / f64::from(u16::MAX) * 2.0 - 1.0;
            (noise * NOTE_PEAK * envelope(f64::from(index) / f64::from(samples))).round() as i16
        })
        .collect()
}

/// A hit: noise fading out over a quarter of a second.
fn hit_sound() -> Vec<i16> {
    noise(HIT_SAMPLES, |share| 1.0 - share)
}

/// A key's click: a short burst of noise that falls away fast.
fn click_sound() -> Vec<i16> {
    noise(CLICK_SAMPLES, |share| (-CLICK_DECAY * share).exp())
}

/// Enter's bell.
#[allow(clippy::cast_possible_truncation, reason = "the peak is a quarter of the samples' range")]
fn bell_sound() -> Vec<i16> {
    (0..BELL_SAMPLES)
        .map(|index| {
            let time = f64::from(index) / f64::from(NOTE_RATE);
            ((2.0 * std::f64::consts::PI * BELL_HZ * time).sin() * NOTE_PEAK * (-NOTE_DECAY * time).exp()).round() as i16
        })
        .collect()
}

/// A short tune: each note a sine that fades.
#[allow(clippy::cast_possible_truncation, reason = "the peak is a quarter of the samples' range")]
fn tune(notes: &[f64]) -> Vec<i16> {
    notes
        .iter()
        .flat_map(|frequency| {
            (0..TUNE_NOTE_SAMPLES).map(move |index| {
                let time = f64::from(index) / f64::from(NOTE_RATE);
                let fade = 1.0 - f64::from(index) / f64::from(TUNE_NOTE_SAMPLES);
                ((2.0 * std::f64::consts::PI * frequency * time).sin() * NOTE_PEAK * fade).round() as i16
            })
        })
        .collect()
}

/// One note's samples: a sine with an exponential decay.
#[allow(clippy::cast_possible_truncation, reason = "the peak is a quarter of the samples' range")]
fn note(frequency: f64) -> Vec<i16> {
    (0..u32::try_from(NOTE_SAMPLES).unwrap_or(0))
        .map(|index| {
            let time = f64::from(index) / f64::from(NOTE_RATE);
            let sample = (2.0 * std::f64::consts::PI * frequency * time).sin() * NOTE_PEAK * (-NOTE_DECAY * time).exp();
            sample.round() as i16
        })
        .collect()
}

/// Every sound of the channel with the handle it takes: the piano's
/// notes and the effects.
pub(super) fn library() -> Vec<(u8, Vec<i16>)> {
    let notes = PIANO.iter().enumerate().filter_map(|(index, (_, _, frequency))| Some((FIRST_HANDLE + u8::try_from(index).ok()?, note(*frequency))));
    let effects = [
        (FIRE_HANDLE, fire_sound()),
        (HIT_HANDLE, hit_sound()),
        (WIN_HANDLE, tune(&WIN_TUNE)),
        (LOSE_HANDLE, tune(&LOSE_TUNE)),
        (CLICK_HANDLE, click_sound()),
        (BELL_HANDLE, bell_sound()),
    ];
    notes.chain(effects).collect()
}
