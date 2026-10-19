//! The interface's sound (the engine's area AUD, decisions 37 and 38): the
//! voice operations the trusted core passes on are applied to one engine
//! mixer for each channel, so every tab is heard and each can be muted, and
//! the operating system's output device plays their sum, through `cpal`.
//!
//! The mixers are shared between the thread that reads the trusted core and
//! the device's audio thread behind a mutex. The reader holds it only to
//! apply one operation; the audio thread holds it only to render one
//! buffer, which never allocates or frees.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::AtomicU16;
#[cfg(target_os = "macos")]
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};

#[cfg(target_os = "macos")]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
#[cfg(target_os = "macos")]
use cpal::{SampleFormat, SizedSample, Stream};
use neoteletext_audio_mix::{Mixer, SoundKey, Start};
use protocol::audio::{Audio, Sound};

#[cfg(target_os = "macos")]
/// The most frames rendered at once: buffers larger than this are rendered
/// in pieces, so the scratch buffer is allocated once, before playing.
const SCRATCH_FRAMES: usize = 8192;

/// The mixers the reader and the audio thread share.
pub type Shared = Arc<Mutex<Mixers>>;

fn key(sound: Sound) -> SoundKey {
    (sound.handle, sound.generation)
}

/// One mixer for each channel with sound, at the output's rate.
pub struct Mixers {
    rate_hz: u32,
    channels: BTreeMap<u32, Mixer>,
}

impl Mixers {
    /// No channel has sound yet.
    #[must_use]
    pub fn new(rate_hz: u32) -> Self {
        Self { rate_hz, channels: BTreeMap::new() }
    }

    /// The channels sounding or about to, muted or not: their tabs show it.
    #[must_use]
    pub fn sounding(&self) -> BTreeSet<u32> {
        self.channels.iter().filter(|(_, mixer)| mixer.active()).map(|(&channel, _)| channel).collect()
    }

    /// Renders the sum of every channel into `out`, using `scratch`, as long
    /// as `out`, for each. It never allocates or frees.
    fn render(&mut self, out: &mut [i16], scratch: &mut [i16]) {
        out.fill(0);
        let Some(scratch) = scratch.get_mut(..out.len()) else { return };
        for mixer in self.channels.values_mut() {
            mixer.render(scratch);
            for (sum, &sample) in out.iter_mut().zip(scratch.iter()) {
                *sum = sum.saturating_add(sample);
            }
        }
    }
}

/// Applies one voice operation of `channel` from the trusted core.
pub fn apply(mixers: &Mutex<Mixers>, channel: u32, audio: Audio) {
    let mut mixers = mixers.lock().unwrap_or_else(PoisonError::into_inner);
    let rate = mixers.rate_hz;
    if matches!(audio, Audio::Reset) {
        // The channel is gone.
        mixers.channels.remove(&channel);
        return;
    }
    let mixer = mixers.channels.entry(channel).or_insert_with(|| Mixer::new(rate));
    match audio {
        Audio::Define { sound, rate_hz, samples } => mixer.define(key(sound), rate_hz, usize::try_from(samples).unwrap_or(0)),
        Audio::Samples { sound, first, pcm } => mixer.samples(key(sound), usize::try_from(first).unwrap_or(usize::MAX), &pcm),
        Audio::Forget(sound) => mixer.forget(key(sound)),
        Audio::Start { channel, sound, gain, fade_ms, looped, duck_percent, after_current } => {
            mixer.start(usize::from(channel), Start { key: key(sound), gain, fade_ms, looped, duck_percent, after_current });
        }
        Audio::Stop { channel, fade_ms } => mixer.stop(usize::from(channel), fade_ms),
        Audio::Gain { channel, gain, fade_ms } => mixer.gain(usize::from(channel), gain, fade_ms),
        Audio::Stream { sound, rate_hz } => mixer.stream(key(sound), rate_hz),
        Audio::StreamSamples { sound, pcm } => mixer.stream_samples(key(sound), &pcm),
        Audio::Mute(muted) => mixer.set_master(100, muted),
        Audio::Reset => {}
    }
}

/// The output device, playing a mixer. Dropped, it stops.
pub struct Speaker {
    /// The mixers the device plays.
    pub mixers: Shared,
    /// The loudest sample rendered since it was last read, for diagnostics.
    pub peak: Arc<AtomicU16>,
    #[cfg(target_os = "macos")]
    _stream: Stream,
}

#[cfg(not(target_os = "macos"))]
impl Speaker {
    /// Sound is built for macOS only so far (decision 37).
    ///
    /// # Errors
    /// Always, on this platform.
    pub fn open() -> Result<Self, String> {
        Err("sound is not built for this platform yet".to_owned())
    }
}

#[cfg(target_os = "macos")]
impl Speaker {
    /// Opens the default output device at its own rate and format.
    ///
    /// # Errors
    /// No output device, or one whose format the mixer cannot feed.
    pub fn open() -> Result<Self, String> {
        let device = cpal::default_host().default_output_device().ok_or("no audio output device")?;
        let supported = device.default_output_config().map_err(|error| format!("the audio output's format: {error}"))?;
        let config = supported.config();
        let mixers: Shared = Arc::new(Mutex::new(Mixers::new(config.sample_rate)));
        let channels = usize::from(config.channels).max(1);
        let peak = Arc::new(AtomicU16::new(0));
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, config, channels, &mixers, &peak),
            SampleFormat::I16 => build::<i16>(&device, config, channels, &mixers, &peak),
            SampleFormat::U16 => build::<u16>(&device, config, channels, &mixers, &peak),
            other => return Err(format!("the audio output's sample format {other} is not supported")),
        }?;
        stream.play().map_err(|error| format!("cannot start the audio output: {error}"))?;
        Ok(Self { mixers, peak, _stream: stream })
    }
}

#[cfg(target_os = "macos")]
fn build<T>(device: &cpal::Device, config: cpal::StreamConfig, channels: usize, mixers: &Shared, peak: &Arc<AtomicU16>) -> Result<Stream, String>
where
    T: SizedSample + OutputSample,
{
    let mixers = Arc::clone(mixers);
    let peak = Arc::clone(peak);
    let mut scratch = vec![0_i16; SCRATCH_FRAMES * 2];
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                let loudest = render(&mixers, data, channels, &mut scratch);
                peak.fetch_max(loudest, Ordering::Relaxed);
            },
            |error| eprintln!("t42-interface: audio output: {error}"),
            None,
        )
        .map_err(|error| format!("cannot open the audio output: {error}"))
}

/// Fills `data`, interleaved frames of `channels`, with the mixers' mono
/// sum in every channel, and returns the loudest sample's magnitude.
/// `scratch` is split in two: the sum, and each mixer's part of it.
pub fn render<T>(mixers: &Mutex<Mixers>, data: &mut [T], channels: usize, scratch: &mut [i16]) -> u16
where
    T: OutputSample,
{
    let mut loudest = 0;
    let channels = channels.max(1);
    let (sum, part) = scratch.split_at_mut(scratch.len() / 2);
    let frames_per_piece = sum.len().max(1);
    let mut mixers = mixers.lock().unwrap_or_else(PoisonError::into_inner);
    for piece in data.chunks_mut(frames_per_piece.saturating_mul(channels)) {
        let frames = piece.len().checked_div(channels).unwrap_or(0);
        let Some(mono) = sum.get_mut(..frames) else { return loudest };
        mixers.render(mono, part);
        loudest = mono.iter().fold(loudest, |loudest, sample| loudest.max(sample.unsigned_abs()));
        for (frame, &sample) in piece.chunks_mut(channels).zip(mono.iter()) {
            for out in frame {
                *out = T::from_i16(sample);
            }
        }
    }
    loudest
}

/// A sample format of the output device.
pub trait OutputSample: Copy {
    /// The sample of a 16-bit value.
    fn from_i16(value: i16) -> Self;
}

impl OutputSample for i16 {
    fn from_i16(value: i16) -> Self {
        value
    }
}

impl OutputSample for f32 {
    fn from_i16(value: i16) -> Self {
        f32::from(value) / 32_768.0
    }
}

impl OutputSample for u16 {
    fn from_i16(value: i16) -> Self {
        value.cast_unsigned() ^ 0x8000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOUND: Sound = Sound { handle: 2, generation: 1 };

    /// Channel `channel` playing a constant `value`.
    fn play(mixers: &Mutex<Mixers>, channel: u32, value: i16) {
        apply(mixers, channel, Audio::Define { sound: SOUND, rate_hz: 8_000, samples: 4000 });
        apply(mixers, channel, Audio::Samples { sound: SOUND, first: 0, pcm: vec![value; 4000] });
        apply(mixers, channel, Audio::Start { channel: 0, sound: SOUND, gain: 100, fade_ms: 5, looped: false, duck_percent: 100, after_current: false });
    }

    fn playing(value: i16) -> Mutex<Mixers> {
        let mixers = Mutex::new(Mixers::new(8_000));
        play(&mixers, 1, value);
        mixers
    }

    #[test]
    fn the_cores_operations_play_through_the_mixer_in_every_channel() {
        let mixer = playing(8_000);
        let mut data = vec![0.0_f32; 2 * 400];
        let mut scratch = vec![0_i16; 128];
        render(&mixer, &mut data, 2, &mut scratch);
        // Settled after the 5 ms ramp: both channels at 8 000 / 32 768.
        let expected = f32::from_i16(8_000);
        assert!((data[700] - expected).abs() < 0.01, "{} against {expected}", data[700]);
        assert_eq!(data[700].to_bits(), data[701].to_bits(), "both channels alike");
    }

    #[test]
    fn a_reset_silences_its_channel_at_once() {
        let mixer = playing(8_000);
        apply(&mixer, 1, Audio::Reset);
        let mut data = vec![1_i16; 100];
        assert_eq!(render(&mixer, &mut data, 1, &mut [0; 64]), 0);
        assert!(data.iter().all(|&sample| sample == 0));
        assert_eq!(mixer.lock().unwrap().sounding(), BTreeSet::new());
    }

    #[test]
    fn a_stop_fades_the_channel_out() {
        let mixer = playing(8_000);
        render(&mixer, &mut vec![0_i16; 200], 1, &mut [0; 512]);
        apply(&mixer, 1, Audio::Stop { channel: 0, fade_ms: 10 });
        let mut data = vec![0_i16; 200];
        render(&mixer, &mut data, 1, &mut [0; 512]);
        assert!(data[0] > 7_000 && data[150] == 0, "{} then {}", data[0], data[150]);
    }

    #[test]
    fn every_channel_is_heard_and_a_muted_one_is_silent_but_still_sounding() {
        let mixers = playing(8_000);
        play(&mixers, 2, 4_000);
        let mut data = vec![0_i16; 400];
        render(&mixers, &mut data, 1, &mut [0; 512]);
        assert!((11_800..=12_200).contains(&data[300]), "both tabs: {}", data[300]);
        apply(&mixers, 1, Audio::Mute(true));
        render(&mixers, &mut data, 1, &mut [0; 512]);
        assert!((3_900..=4_100).contains(&data[300]), "tab 1 muted: {}", data[300]);
        assert_eq!(mixers.lock().unwrap().sounding(), BTreeSet::from([1, 2]), "a muted tab still shows it is playing");
        apply(&mixers, 1, Audio::Mute(false));
        render(&mixers, &mut data, 1, &mut [0; 512]);
        assert!((11_800..=12_200).contains(&data[300]), "unmuted: {}", data[300]);
    }

    #[test]
    fn a_live_sound_plays_once_its_buffer_fills() {
        let mixers = Mutex::new(Mixers::new(8_000));
        apply(&mixers, 3, Audio::Stream { sound: SOUND, rate_hz: 8_000 });
        apply(&mixers, 3, Audio::Start { channel: 0, sound: SOUND, gain: 100, fade_ms: 5, looped: false, duck_percent: 100, after_current: false });
        apply(&mixers, 3, Audio::StreamSamples { sound: SOUND, pcm: vec![6_000; 4_000] });
        let mut data = vec![0_i16; 400];
        render(&mixers, &mut data, 1, &mut [0; 512]);
        assert!((5_800..=6_200).contains(&data[300]), "{}", data[300]);
    }

    /// Plays a second of a tone through the real default output device.
    /// It needs one, so it runs only when asked: `cargo test -- --ignored`.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "needs an audio output device"]
    fn a_tone_plays_through_the_default_output_device() {
        let speaker = Speaker::open().expect("an output device");
        let rate: i32 = 16_000;
        // A triangle wave of about 889 Hz: 18 samples a period.
        let tone: Vec<i16> = (0..rate)
            .map(|n| {
                let phase = i16::try_from(n % 18).unwrap();
                (if phase < 9 { phase } else { 18 - phase }) * 1_300 - 5_850
            })
            .collect();
        apply(&speaker.mixers, 1, Audio::Define { sound: SOUND, rate_hz: 16_000, samples: 16_000 });
        apply(&speaker.mixers, 1, Audio::Samples { sound: SOUND, first: 0, pcm: tone });
        apply(&speaker.mixers, 1, Audio::Start { channel: 0, sound: SOUND, gain: 50, fade_ms: 20, looped: false, duck_percent: 100, after_current: false });
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(speaker.mixers.lock().unwrap().sounding(), BTreeSet::from([1]), "still playing after 300 ms: the device is pulling samples");
        std::thread::sleep(std::time::Duration::from_millis(1000));
        assert_eq!(speaker.mixers.lock().unwrap().sounding(), BTreeSet::new(), "finished: the device consumed the whole second");
    }
}
