//! The operating system's output device, through `cpal`: opened at its
//! own rate and format, run only while a channel sounds, and fed the
//! mixers' sum.

use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, PoisonError};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample, Stream};

use super::{Mixers, SCRATCH_FRAMES, Shared};

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
