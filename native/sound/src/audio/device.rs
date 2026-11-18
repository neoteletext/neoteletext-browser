//! The operating system's output device, through `cpal`: opened at its
//! own rate and format, run only while a channel sounds, and fed the
//! mixers' sum.

use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, PoisonError};

use std::sync::mpsc::SyncSender;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, SampleFormat, SizedSample, Stream, SupportedBufferSize};

use super::buffer::{Offered, Size};
use super::{Mixers, SCRATCH_FRAMES, Shared, Told};

/// The output device, playing the mixers while a channel sounds. Dropped,
/// it stops.
pub struct Speaker {
    /// The mixers the device plays.
    pub mixers: Shared,
    /// The loudest sample rendered since it was last read, for diagnostics.
    pub peak: Arc<AtomicU16>,
    /// Wakes for [`watch`], taken once by the thread that watches.
    pub wakes: Option<Receiver<()>>,
    /// What the audio thread metered and stopped (page-rules.md RULE-33,
    /// RULE-62), taken once by the thread that tells the trusted core.
    pub told: Option<Receiver<Told>>,
    stream: Stream,
    playing: bool,
    /// The device, its format and its own configuration, for opening it
    /// again with another buffer (page-rules.md RULE-97).
    device: cpal::Device,
    format: SampleFormat,
    config: cpal::StreamConfig,
    /// What the device offers for its buffer, and the size it was last
    /// opened with.
    offered: Offered,
    size: Size,
    /// Where the audio thread says a buffer took longer than its own time
    /// or the device failed (RULE-97 item 3).
    trouble: SyncSender<Told>,
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
        let offered = match supported.buffer_size() {
            SupportedBufferSize::Range { min, max } => Offered::Range(*min, *max),
            SupportedBufferSize::Unknown => Offered::Own,
        };
        let (wake, wakes) = std::sync::mpsc::sync_channel(1);
        let (tell, told) = std::sync::mpsc::sync_channel(super::TOLD_QUEUE);
        let mut mixers = Mixers::new(config.sample_rate);
        mixers.wake = Some(wake);
        mixers.tell(tell.clone());
        let mixers: Shared = Arc::new(Mutex::new(mixers));
        let peak = Arc::new(AtomicU16::new(0));
        let format = supported.sample_format();
        let size = super::buffer::usual(offered, config.sample_rate);
        let stream = build_stream(&device, format, sized(&config, size), &mixers, &peak, &tell)?;
        // Some platforms start a stream as it is built; this one waits.
        let _ = stream.pause();
        Ok(Self { mixers, peak, wakes: Some(wakes), told: Some(told), stream, playing: false, device, format, config, offered, size, trouble: tell })
    }

    /// What the device offers for its buffer, and its rate.
    #[must_use]
    pub const fn offered(&self) -> (Offered, u32) {
        (self.offered, self.config.sample_rate)
    }

    /// The device buffer it was last opened with.
    #[must_use]
    pub const fn size(&self) -> Size {
        self.size
    }

    /// Opens the device again with a buffer of `size` (RULE-97 item 2):
    /// between two buffers, the mixers and every voice in them kept as they
    /// are, so no stream restarts and no instrument is reset, and the next
    /// buffer goes on from where the last ended. Running again if it ran.
    ///
    /// # Errors
    /// When the device will not open at that size; the stream it had then
    /// plays on.
    pub fn reopen(&mut self, size: Size) -> Result<(), String> {
        if size == self.size {
            return Ok(());
        }
        let opened = build_stream(&self.device, self.format, sized(&self.config, size), &self.mixers, &self.peak, &self.trouble);
        let stream = opened?;
        let _ = stream.pause();
        // The old stream goes before the new one plays: one buffer is never
        // filled by both.
        self.stream = stream;
        self.size = size;
        if self.playing
            && let Err(error) = self.stream.play()
        {
            self.playing = false;
            return Err(format!("the audio output did not start again: {error}"));
        }
        Ok(())
    }

    /// Runs the device while `playing`, and stops it otherwise. A device
    /// that cannot stop keeps running: sound matters more than the wakeups.
    /// Returns whether the device started or stopped.
    pub fn set_playing(&mut self, playing: bool) -> bool {
        if playing == self.playing {
            return false;
        }
        let result = if playing { self.stream.play() } else { self.stream.pause() };
        match result {
            Ok(()) => {
                self.playing = playing;
                true
            }
            Err(error) => {
                eprintln!("neoteletext-sound: audio output: {error}");
                false
            }
        }
    }
}

/// The device's configuration with a buffer of `size`.
fn sized(config: &cpal::StreamConfig, size: Size) -> cpal::StreamConfig {
    let mut config = *config;
    config.buffer_size = match size {
        Size::Own => BufferSize::Default,
        Size::Frames(frames) => BufferSize::Fixed(frames),
    };
    config
}

fn build_stream(
    device: &cpal::Device,
    format: SampleFormat,
    config: cpal::StreamConfig,
    mixers: &Shared,
    peak: &Arc<AtomicU16>,
    trouble: &SyncSender<Told>,
) -> Result<Stream, String> {
    match format {
        SampleFormat::F32 => build::<f32>(device, config, mixers, peak, trouble),
        SampleFormat::I16 => build::<i16>(device, config, mixers, peak, trouble),
        SampleFormat::U16 => build::<u16>(device, config, mixers, peak, trouble),
        other => Err(format!("the audio output's sample format {other} is not supported")),
    }
}

/// Microseconds in a second.
const MICROSECONDS_PER_SECOND: u128 = 1_000_000;

fn build<T>(device: &cpal::Device, config: cpal::StreamConfig, mixers: &Shared, peak: &Arc<AtomicU16>, trouble: &SyncSender<Told>) -> Result<Stream, String>
where
    T: SizedSample + OutputSample,
{
    let channels = usize::from(config.channels).max(1);
    let rate_hz = u128::from(config.sample_rate);
    let mixers = Arc::clone(mixers);
    let peak = Arc::clone(peak);
    let (late, failed) = (trouble.clone(), trouble.clone());
    let mut scratch = vec![0_i16; SCRATCH_FRAMES * 2];
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                let started = std::time::Instant::now();
                let loudest = render(&mixers, data, channels, &mut scratch);
                peak.fetch_max(loudest, Ordering::Relaxed);
                // RULE-66: the first sample of a note that is not silence
                // is handed to the device as this returns; and the size the
                // device fills, which RULE-64's bound follows.
                #[cfg(feature = "timed-test")]
                {
                    let frames = data.len().checked_div(channels).unwrap_or(0);
                    if super::local::NOTE_SOUNDED.swap(false, Ordering::Relaxed) {
                        protocol::timed_test::mark("sound", "sample", std::time::Instant::now(), format_args!("{frames}"));
                    }
                }
                // RULE-97 item 3: a buffer that took longer to fill than
                // its own time is an overrun, told without waiting.
                let frames = u128::try_from(data.len().checked_div(channels).unwrap_or(0)).unwrap_or(u128::MAX);
                let own_us = frames.saturating_mul(MICROSECONDS_PER_SECOND).checked_div(rate_hz).unwrap_or(u128::MAX);
                if started.elapsed().as_micros() > own_us {
                    let _ = late.try_send(Told::DeviceTrouble);
                }
            },
            move |error| {
                // An underrun, or any failure the device reports.
                eprintln!("neoteletext-sound: audio output: {error}");
                let _ = failed.try_send(Told::DeviceTrouble);
            },
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
