//! The interface's sound (the engine's area AUD, decision 37): the voice
//! operations the trusted core passes on are applied to the engine's mixer,
//! and the operating system's output device plays what it renders, through
//! `cpal`.
//!
//! The mixer is shared between the thread that reads the trusted core and
//! the device's audio thread behind a mutex. The reader holds it only to
//! apply one operation; the audio thread holds it only to render one
//! buffer, which never allocates or frees.

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

/// A mixer the reader and the audio thread share.
pub type Shared = Arc<Mutex<Mixer>>;

fn key(sound: Sound) -> SoundKey {
    (sound.handle, sound.generation)
}

/// Applies one voice operation from the trusted core.
pub fn apply(mixer: &Mutex<Mixer>, audio: Audio) {
    let mut mixer = mixer.lock().unwrap_or_else(PoisonError::into_inner);
    match audio {
        Audio::Define { sound, rate_hz, samples } => mixer.define(key(sound), rate_hz, usize::try_from(samples).unwrap_or(0)),
        Audio::Samples { sound, first, pcm } => mixer.samples(key(sound), usize::try_from(first).unwrap_or(usize::MAX), &pcm),
        Audio::Forget(sound) => mixer.forget(key(sound)),
        Audio::Start { channel, sound, gain, fade_ms, looped, duck_percent, after_current } => {
            mixer.start(usize::from(channel), Start { key: key(sound), gain, fade_ms, looped, duck_percent, after_current });
        }
        Audio::Stop { channel, fade_ms } => mixer.stop(usize::from(channel), fade_ms),
        Audio::Gain { channel, gain, fade_ms } => mixer.gain(usize::from(channel), gain, fade_ms),
        Audio::Reset => mixer.clear(),
    }
}

/// The output device, playing a mixer. Dropped, it stops.
pub struct Speaker {
    /// The mixer the device plays.
    pub mixer: Shared,
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
        let mixer: Shared = Arc::new(Mutex::new(Mixer::new(config.sample_rate)));
        let channels = usize::from(config.channels).max(1);
        let peak = Arc::new(AtomicU16::new(0));
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, config, channels, &mixer, &peak),
            SampleFormat::I16 => build::<i16>(&device, config, channels, &mixer, &peak),
            SampleFormat::U16 => build::<u16>(&device, config, channels, &mixer, &peak),
            other => return Err(format!("the audio output's sample format {other} is not supported")),
        }?;
        stream.play().map_err(|error| format!("cannot start the audio output: {error}"))?;
        Ok(Self { mixer, peak, _stream: stream })
    }
}

#[cfg(target_os = "macos")]
fn build<T>(device: &cpal::Device, config: cpal::StreamConfig, channels: usize, mixer: &Shared, peak: &Arc<AtomicU16>) -> Result<Stream, String>
where
    T: SizedSample + OutputSample,
{
    let mixer = Arc::clone(mixer);
    let peak = Arc::clone(peak);
    let mut scratch = vec![0_i16; SCRATCH_FRAMES];
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                let loudest = render(&mixer, data, channels, &mut scratch);
                peak.fetch_max(loudest, Ordering::Relaxed);
            },
            |error| eprintln!("t42-interface: audio output: {error}"),
            None,
        )
        .map_err(|error| format!("cannot open the audio output: {error}"))
}

/// Fills `data`, interleaved frames of `channels`, with the mixer's mono
/// output in every channel, and returns the loudest sample's magnitude.
pub fn render<T>(mixer: &Mutex<Mixer>, data: &mut [T], channels: usize, scratch: &mut [i16]) -> u16
where
    T: OutputSample,
{
    let mut loudest = 0;
    let channels = channels.max(1);
    let frames_per_piece = scratch.len().max(1);
    let mut mixer = mixer.lock().unwrap_or_else(PoisonError::into_inner);
    for piece in data.chunks_mut(frames_per_piece.saturating_mul(channels)) {
        let frames = piece.len().checked_div(channels).unwrap_or(0);
        let Some(mono) = scratch.get_mut(..frames) else { return loudest };
        mixer.render(mono);
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

    fn playing(value: i16) -> Mutex<Mixer> {
        let mixer = Mutex::new(Mixer::new(8_000));
        apply(&mixer, Audio::Define { sound: SOUND, rate_hz: 8_000, samples: 4000 });
        apply(&mixer, Audio::Samples { sound: SOUND, first: 0, pcm: vec![value; 4000] });
        apply(&mixer, Audio::Start { channel: 0, sound: SOUND, gain: 100, fade_ms: 5, looped: false, duck_percent: 100, after_current: false });
        mixer
    }

    #[test]
    fn the_cores_operations_play_through_the_mixer_in_every_channel() {
        let mixer = playing(8_000);
        let mut data = vec![0.0_f32; 2 * 400];
        let mut scratch = vec![0_i16; 64];
        render(&mixer, &mut data, 2, &mut scratch);
        // Settled after the 5 ms ramp: both channels at 8 000 / 32 768.
        let expected = f32::from_i16(8_000);
        assert!((data[700] - expected).abs() < 0.01, "{} against {expected}", data[700]);
        assert_eq!(data[700].to_bits(), data[701].to_bits(), "both channels alike");
    }

    #[test]
    fn a_reset_silences_everything_at_once() {
        let mixer = playing(8_000);
        apply(&mixer, Audio::Reset);
        let mut data = vec![1_i16; 100];
        assert_eq!(render(&mixer, &mut data, 1, &mut [0; 32]), 0);
        assert!(data.iter().all(|&sample| sample == 0));
    }

    #[test]
    fn a_stop_fades_the_channel_out() {
        let mixer = playing(8_000);
        render(&mixer, &mut vec![0_i16; 200], 1, &mut [0; 256]);
        apply(&mixer, Audio::Stop { channel: 0, fade_ms: 10 });
        let mut data = vec![0_i16; 200];
        render(&mixer, &mut data, 1, &mut [0; 256]);
        assert!(data[0] > 7_000 && data[150] == 0, "{} then {}", data[0], data[150]);
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
        apply(&speaker.mixer, Audio::Define { sound: SOUND, rate_hz: 16_000, samples: 16_000 });
        apply(&speaker.mixer, Audio::Samples { sound: SOUND, first: 0, pcm: tone });
        apply(&speaker.mixer, Audio::Start { channel: 0, sound: SOUND, gain: 50, fade_ms: 20, looped: false, duck_percent: 100, after_current: false });
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(speaker.mixer.lock().unwrap().active(), "still playing after 300 ms: the device is pulling samples");
        std::thread::sleep(std::time::Duration::from_millis(1000));
        assert!(!speaker.mixer.lock().unwrap().active(), "finished: the device consumed the whole second");
    }
}
