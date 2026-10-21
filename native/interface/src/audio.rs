//! The interface's sound (the engine's area AUD, decisions 37, 38 and 40): the
//! voice operations the trusted core passes on are applied to one engine
//! mixer for each channel, so every tab is heard and each can be muted, and
//! the operating system's output device plays their sum, through `cpal`.
//!
//! The mixers are shared between the thread that reads the trusted core and
//! the device's audio thread behind a mutex. The reader holds it only to
//! apply one operation; the audio thread holds it only to render one
//! buffer, which never allocates or frees. The longest an operation holds
//! it is defining the largest sound, 0.37 ms on 21 October 2026, against
//! a device buffer of about 10 ms. The audio thread waits for the lock
//! rather than skipping a buffer: a skipped buffer is a click, and the
//! wait is bounded by that measurement. What the reader frees (a channel's
//! whole mixer) it frees after letting go.
//!
//! The device runs only while a channel sounds. Whichever thread finds
//! that the channels went from silent to sounding or back wakes a watcher
//! through a channel (decision 46: a program waits on what it needs, never
//! by looking again on a timer); the watcher tells the window, which
//! starts or stops the device. Idle, the browser then costs the audio
//! thread nothing: before this, the device ran all the time and woke the
//! process about a hundred times a second (measured 21 October 2026).
//!
//! The settings' sound and volume (requirement 19.4) are each mixer's
//! master: off silences every channel, and the volume scales them all.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, PoisonError};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample, Stream};
use neoteletext_audio_mix::{Mixer, SoundKey, Start};
use protocol::audio::{Audio, Sound};

/// The most frames rendered at once: buffers larger than this are rendered
/// in pieces, so the scratch buffer is allocated once, before playing.
const SCRATCH_FRAMES: usize = 8192;

/// The mixers the reader and the audio thread share.
pub type Shared = Arc<Mutex<Mixers>>;

fn key(sound: Sound) -> SoundKey {
    (sound.handle, sound.generation)
}

/// The loudest volume: the engine mixer's master is a percentage.
const MAX_VOLUME: u8 = 100;
const _: () = assert!(MAX_VOLUME == protocol::settings::MAX_VOLUME);

/// One mixer for each channel with sound, at the output's rate.
pub struct Mixers {
    rate_hz: u32,
    channels: BTreeMap<u32, Mixer>,
    /// The settings: page sounds play, and their volume.
    sound: bool,
    volume: u8,
    /// The channels whose tabs are muted.
    muted: BTreeSet<u32>,
    /// Whether any channel sounded when last looked.
    active: bool,
    /// Woken when that changes. It holds one wake, and a wake that finds
    /// one waiting is not needed: the watcher looks afresh each time.
    wake: Option<SyncSender<()>>,
}

impl Mixers {
    /// No channel has sound yet; sound is on at full volume until the
    /// settings say otherwise.
    #[must_use]
    pub fn new(rate_hz: u32) -> Self {
        Self { rate_hz, channels: BTreeMap::new(), sound: true, volume: MAX_VOLUME, muted: BTreeSet::new(), active: false, wake: None }
    }

    /// Wakes the watcher when the channels went from silent to sounding or
    /// back. Called with the lock held, on the audio thread too: it never
    /// blocks or allocates.
    fn note_activity(&mut self) {
        let active = self.channels.values().any(Mixer::active);
        if active == self.active {
            return;
        }
        self.active = active;
        if let Some(wake) = &self.wake
            && let Err(TrySendError::Disconnected(())) = wake.try_send(())
        {
            self.wake = None;
        }
    }

    /// Each channel's master: the volume, silent when sound is off or the
    /// tab is muted.
    fn master(&mut self, channel: u32) {
        let silent = !self.sound || self.muted.contains(&channel);
        if let Some(mixer) = self.channels.get_mut(&channel) {
            mixer.set_master(self.volume, silent);
        }
    }

    /// The channels sounding or about to, muted or not: their tabs show it.
    #[must_use]
    pub fn sounding(&self) -> BTreeSet<u32> {
        self.channels.iter().filter(|(_, mixer)| mixer.active()).map(|(&channel, _)| channel).collect()
    }

    /// Renders the sum of every channel into `out`, using `scratch`, as long
    /// as `out`, for each. It never allocates or frees. A channel with
    /// nothing playing or about to is silent and is not rendered: a mixer
    /// costs its work for every sample whether or not it sounds, so a few
    /// hundred tabs that once played a sound would otherwise take a whole
    /// processor core on the audio thread.
    fn render(&mut self, out: &mut [i16], scratch: &mut [i16]) {
        out.fill(0);
        let Some(scratch) = scratch.get_mut(..out.len()) else { return };
        for mixer in self.channels.values_mut().filter(|mixer| mixer.active()) {
            mixer.render(scratch);
            for (sum, &sample) in out.iter_mut().zip(scratch.iter()) {
                *sum = sum.saturating_add(sample);
            }
        }
        // A sound ends here, on the audio thread.
        self.note_activity();
    }
}

/// Waits for the channels to go from silent to sounding or back, and tells
/// `changed` which channels sound, muted or not, each time that set
/// changes. Returns when the mixers are gone or `changed` says to stop.
pub fn watch(mixers: &Mutex<Mixers>, wakes: &Receiver<()>, mut changed: impl FnMut(BTreeSet<u32>) -> bool) {
    let mut last = BTreeSet::new();
    while wakes.recv().is_ok() {
        let sounding = mixers.lock().unwrap_or_else(PoisonError::into_inner).sounding();
        if sounding != last {
            last.clone_from(&sounding);
            if !changed(sounding) {
                return;
            }
        }
    }
}

/// The settings' sound and volume, for every channel (requirement 19.4).
pub fn set_sound(mixers: &Mutex<Mixers>, sound: bool, volume: u8) {
    let mut mixers = mixers.lock().unwrap_or_else(PoisonError::into_inner);
    mixers.sound = sound;
    mixers.volume = volume.min(MAX_VOLUME);
    let channels: Vec<u32> = mixers.channels.keys().copied().collect();
    for channel in channels {
        mixers.master(channel);
    }
}

/// Applies one voice operation of `channel` from the trusted core.
pub fn apply(mixers: &Mutex<Mixers>, channel: u32, audio: Audio) {
    let mut mixers = mixers.lock().unwrap_or_else(PoisonError::into_inner);
    let rate = mixers.rate_hz;
    match audio {
        // The channel is gone. Its mixer, with every sound it held, is
        // freed once the audio thread can have the lock again.
        Audio::Reset => {
            let gone = mixers.channels.remove(&channel);
            mixers.muted.remove(&channel);
            mixers.note_activity();
            drop(mixers);
            drop(gone);
            return;
        }
        Audio::Mute(muted) => {
            if muted {
                mixers.muted.insert(channel);
            } else {
                mixers.muted.remove(&channel);
            }
        }
        _ => {}
    }
    if let std::collections::btree_map::Entry::Vacant(vacant) = mixers.channels.entry(channel) {
        vacant.insert(Mixer::new(rate));
        mixers.master(channel);
    }
    if matches!(audio, Audio::Mute(_)) {
        mixers.master(channel);
        return;
    }
    let Some(mixer) = mixers.channels.get_mut(&channel) else { return };
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
        Audio::Mute(_) | Audio::Reset => {}
    }
    // A sound starts here, on the reader's thread, while the device may
    // be stopped.
    mixers.note_activity();
}

/// The output device, playing the mixers while a channel sounds. Dropped,
/// it stops.
pub struct Speaker {
    /// The mixers the device plays.
    pub mixers: Shared,
    /// The loudest sample rendered since it was last read, for diagnostics.
    pub peak: Arc<AtomicU16>,
    /// Wakes for [`watch`], taken once by the thread that watches.
    pub wakes: Option<Receiver<()>>,
    stream: Stream,
    playing: bool,
}

impl Speaker {
    /// Opens the default output device at its own rate and format, stopped
    /// until a channel sounds.
    ///
    /// # Errors
    /// No output device, or one whose format the mixer cannot feed.
    pub fn open() -> Result<Self, String> {
        let device = cpal::default_host().default_output_device().ok_or("no audio output device")?;
        let supported = device.default_output_config().map_err(|error| format!("the audio output's format: {error}"))?;
        let config = supported.config();
        let (wake, wakes) = std::sync::mpsc::sync_channel(1);
        let mut mixers = Mixers::new(config.sample_rate);
        mixers.wake = Some(wake);
        let mixers: Shared = Arc::new(Mutex::new(mixers));
        let channels = usize::from(config.channels).max(1);
        let peak = Arc::new(AtomicU16::new(0));
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, config, channels, &mixers, &peak),
            SampleFormat::I16 => build::<i16>(&device, config, channels, &mixers, &peak),
            SampleFormat::U16 => build::<u16>(&device, config, channels, &mixers, &peak),
            other => return Err(format!("the audio output's sample format {other} is not supported")),
        }?;
        // Some platforms start a stream as it is built; this one waits.
        let _ = stream.pause();
        Ok(Self { mixers, peak, wakes: Some(wakes), stream, playing: false })
    }

    /// Runs the device while `playing`, and stops it otherwise. A device
    /// that cannot stop keeps running: sound matters more than the wakeups.
    pub fn set_playing(&mut self, playing: bool) {
        if playing == self.playing {
            return;
        }
        let result = if playing { self.stream.play() } else { self.stream.pause() };
        match result {
            Ok(()) => self.playing = playing,
            Err(error) => eprintln!("neoteletext-interface: audio output: {error}"),
        }
    }
}

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
            |error| eprintln!("neoteletext-interface: audio output: {error}"),
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
    fn silent_channels_cost_the_audio_thread_nothing() {
        // Tabs that once had a sound and now play none: each mixer rendered
        // anyway cost 2 ms of processor time a second of audio (release
        // build, 21 October 2026), so these 500 took a whole core, and many
        // times that in a debug build.
        const TABS: u32 = 500;
        const RATE: u32 = 48_000;
        const PIECE: usize = 512;
        const BUDGET: std::time::Duration = std::time::Duration::from_millis(100);
        let mixers = Mutex::new(Mixers::new(RATE));
        for channel in 0..TABS {
            apply(&mixers, channel, Audio::Define { sound: SOUND, rate_hz: RATE, samples: 1 });
        }
        let mut data = vec![0_i16; PIECE];
        let mut scratch = vec![0_i16; 2 * PIECE];
        let started = std::time::Instant::now();
        for _ in 0..usize::try_from(RATE).unwrap() / PIECE {
            render(&mixers, &mut data, 1, &mut scratch);
        }
        let spent = started.elapsed();
        assert!(spent < BUDGET, "a second of audio took {spent:?} for {TABS} silent channels");
        assert!(data.iter().all(|&sample| sample == 0));
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

    /// Requirement 19.4: sound off silences every channel, a muted tab
    /// stays muted, and the volume scales them.
    #[test]
    fn the_settings_silence_and_scale_every_channel() {
        // Five seconds each, longer than every render below together.
        const SAMPLES: u32 = 40_000;
        let play = |mixers: &Mutex<Mixers>, channel: u32, value: i16| {
            apply(mixers, channel, Audio::Define { sound: SOUND, rate_hz: 8_000, samples: SAMPLES });
            apply(mixers, channel, Audio::Samples { sound: SOUND, first: 0, pcm: vec![value; usize::try_from(SAMPLES).unwrap()] });
            apply(mixers, channel, Audio::Start { channel: 0, sound: SOUND, gain: 100, fade_ms: 5, looped: false, duck_percent: 100, after_current: false });
        };
        let mixers = Mutex::new(Mixers::new(8_000));
        play(&mixers, 1, 8_000);
        play(&mixers, 2, 4_000);
        let mut data = vec![0_i16; 400];
        let mut heard = |sound: bool, volume: u8| {
            set_sound(&mixers, sound, volume);
            render(&mixers, &mut data, 1, &mut [0; 512]);
            data[300]
        };
        assert_eq!(heard(false, 100), 0, "sound off");
        for volume in [0_u8, 10, 25, 50, 80, 100] {
            let expected = 12_000 * i32::from(volume) / 100;
            let sample = i32::from(heard(true, volume));
            assert!((sample - expected).abs() <= 120, "volume {volume}: {sample} against {expected}");
        }
        apply(&mixers, 1, Audio::Mute(true));
        assert!((3_900..=4_100).contains(&heard(true, 100)), "tab 1 still muted");
        assert!((1_900..=2_100).contains(&heard(true, 50)));
        assert_eq!(heard(false, 50), 0);
        play(&mixers, 3, 4_000);
        assert_eq!(heard(false, 50), 0, "a channel that starts later is silent too");
        assert!((3_900..=4_100).contains(&heard(true, 50)), "and scaled: {}", data[300]);
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

    /// The watcher is woken once when a sound starts and once when the
    /// last ends, never while sounds come and go in between, and it
    /// reports the sounding channels each time they change.
    #[test]
    fn the_watcher_is_woken_only_when_silence_starts_or_ends() {
        let (wake, wakes) = std::sync::mpsc::sync_channel(1);
        let mut mixers = Mixers::new(8_000);
        mixers.wake = Some(wake);
        let mixers = Mutex::new(mixers);
        apply(&mixers, 1, Audio::Define { sound: SOUND, rate_hz: 8_000, samples: 4000 });
        assert!(wakes.try_recv().is_err(), "a defined sound does not sound");
        play(&mixers, 1, 8_000);
        assert!(wakes.try_recv().is_ok(), "the first sound wakes the watcher");
        play(&mixers, 2, 4_000);
        assert!(wakes.try_recv().is_err(), "a second channel sounding changes nothing for the device");
        // The sounds last half a second: a quarter rendered, then another.
        let mut data = vec![0_i16; 2_000];
        render(&mixers, &mut data, 1, &mut [0; 512]);
        assert!(wakes.try_recv().is_err(), "still sounding after a quarter of a second");
        render(&mixers, &mut data, 1, &mut [0; 512]);
        render(&mixers, &mut data, 1, &mut [0; 512]);
        assert!(wakes.try_recv().is_ok(), "the audio thread wakes the watcher as the last sound ends");
        assert_eq!(mixers.lock().unwrap().sounding(), BTreeSet::new());
        // The watcher reports each change of the set, and stops when told.
        play(&mixers, 3, 1_000);
        let reported = Mutex::new(Vec::new());
        watch(&mixers, &wakes, |sounding| {
            reported.lock().unwrap().push(sounding);
            false
        });
        assert_eq!(reported.into_inner().unwrap(), [BTreeSet::from([3])]);
    }

    /// Plays a second of a tone through the real default output device,
    /// which runs only while the tone plays. It needs a device, so it runs
    /// only when asked: `cargo test -- --ignored`.
    #[test]
    #[ignore = "needs an audio output device"]
    fn a_tone_plays_through_the_default_output_device() {
        let mut speaker = Speaker::open().expect("an output device");
        let wakes = speaker.wakes.take().unwrap();
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
        assert!(wakes.recv_timeout(std::time::Duration::from_millis(100)).is_ok(), "the start wakes the watcher");
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(speaker.mixers.lock().unwrap().sounding(), BTreeSet::from([1]), "the device is stopped: nothing was consumed");
        speaker.set_playing(true);
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(speaker.mixers.lock().unwrap().sounding(), BTreeSet::from([1]), "still playing after 300 ms: the device is pulling samples");
        assert!(wakes.recv_timeout(std::time::Duration::from_millis(1500)).is_ok(), "the end wakes the watcher");
        assert_eq!(speaker.mixers.lock().unwrap().sounding(), BTreeSet::new(), "finished: the device consumed the whole second");
        speaker.set_playing(false);
    }
}
