//! The operating system's output device, through the helper's own
//! stream (`sound-output`, DEC-7168): opened at its own rate, in one
//! format, signed 16-bit samples in two channels, run only while a channel
//! sounds, and fed the mixers' sum.

use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, PoisonError};

use std::sync::mpsc::SyncSender;

use sound_output::{CHANNELS, Described, Granted, Output};

use super::buffer::{Offered, Size};
use super::limiter::narrowed;
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
    stream: Output,
    playing: bool,
    /// The device as it was described, for opening it again with another
    /// buffer (page-rules.md RULE-97).
    described: Described,
    /// What the device offers for its buffer, and the size it was last
    /// opened with.
    offered: Offered,
    size: Size,
    /// Where the audio thread says a buffer took longer than its own time
    /// or the device failed (RULE-97 item 3).
    trouble: SyncSender<Told>,
}

impl Speaker {
    /// Opens the default output device at its own rate, stopped until a
    /// channel sounds.
    ///
    /// # Errors
    /// No output device, or one that cannot play the stream's format.
    pub fn open() -> Result<Self, String> {
        let described = sound_output::describe().map_err(|error| format!("no audio output device: {error}"))?;
        let offered = match described.granted {
            Granted::Range(least, most) => Offered::Range(least, most),
            Granted::Own => Offered::Own,
        };
        let (wake, wakes) = std::sync::mpsc::sync_channel(1);
        let (tell, told) = std::sync::mpsc::sync_channel(super::TOLD_QUEUE);
        let mut mixers = Mixers::new(described.rate_hz);
        mixers.wake = Some(wake);
        mixers.tell(tell.clone());
        let mixers: Shared = Arc::new(Mutex::new(mixers));
        let peak = Arc::new(AtomicU16::new(0));
        let size = super::buffer::usual(offered, described.rate_hz);
        let stream = build(&described, size, &mixers, &peak, &tell)?;
        Ok(Self { mixers, peak, wakes: Some(wakes), told: Some(told), stream, playing: false, described, offered, size, trouble: tell })
    }

    /// What the device offers for its buffer, and its rate.
    #[must_use]
    pub const fn offered(&self) -> (Offered, u32) {
        (self.offered, self.described.rate_hz)
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
        let stream = build(&self.described, size, &self.mixers, &self.peak, &self.trouble)?;
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

/// Microseconds in a second.
const MICROSECONDS_PER_SECOND: u128 = 1_000_000;

/// A stream on the device with a buffer of `size`, stopped, filled from
/// `mixers`.
fn build(described: &Described, size: Size, mixers: &Shared, peak: &Arc<AtomicU16>, trouble: &SyncSender<Told>) -> Result<Output, String> {
    let rate_hz = u128::from(described.rate_hz);
    let mixers = Arc::clone(mixers);
    let peak = Arc::clone(peak);
    let (late, failed) = (trouble.clone(), trouble.clone());
    let mut scratch = vec![0_i32; SCRATCH_FRAMES * 2];
    let size = match size {
        Size::Own => sound_output::Size::Own,
        Size::Frames(frames) => sound_output::Size::Frames(frames),
    };
    let fill = Box::new(move |data: &mut [i16]| {
        let started = std::time::Instant::now();
        let loudest = render(&mixers, data, CHANNELS, &mut scratch);
        peak.fetch_max(loudest, Ordering::Relaxed);
        // RULE-66: the first sample of a note that is not silence is handed
        // to the device as this returns; and the size the device fills,
        // which RULE-64's bound follows.
        #[cfg(feature = "timed-test")]
        {
            let frames = data.len().checked_div(CHANNELS).unwrap_or(0);
            if super::local::NOTE_SOUNDED.swap(false, Ordering::Relaxed) {
                protocol::timed_test::mark("sound", "sample", std::time::Instant::now(), format_args!("{frames}"));
            }
        }
        // RULE-97 item 3: a buffer that took longer to fill than its own
        // time is an overrun, told without waiting.
        let frames = u128::try_from(data.len().checked_div(CHANNELS).unwrap_or(0)).unwrap_or(u128::MAX);
        let own_us = frames.saturating_mul(MICROSECONDS_PER_SECOND).checked_div(rate_hz).unwrap_or(u128::MAX);
        if started.elapsed().as_micros() > own_us {
            let _ = late.try_send(Told::DeviceTrouble);
        }
    });
    let trouble = Box::new(move |error: &str| {
        // An underrun, or any failure the device reports.
        eprintln!("neoteletext-sound: audio output: {error}");
        let _ = failed.try_send(Told::DeviceTrouble);
    });
    Output::open(described, size, fill, trouble).map_err(|error| format!("cannot open the audio output: {error}"))
}

/// Fills `data`, interleaved frames of `channels`, with the mixers' mono
/// sum, limited (REQ-6933), in every channel, and returns the loudest
/// sample's magnitude. `scratch` is split in two: the sum, and each
/// mixer's part of it.
pub fn render(mixers: &Mutex<Mixers>, data: &mut [i16], channels: usize, scratch: &mut [i32]) -> u16 {
    let mut loudest = 0;
    let channels = channels.max(1);
    let (sum, part) = scratch.split_at_mut(scratch.len() / 2);
    let frames_per_piece = sum.len().max(1);
    let mut mixers = mixers.lock().unwrap_or_else(PoisonError::into_inner);
    for piece in data.chunks_mut(frames_per_piece.saturating_mul(channels)) {
        let frames = piece.len().checked_div(channels).unwrap_or(0);
        let Some(mono) = sum.get_mut(..frames) else { return loudest };
        mixers.render(mono, part);
        for (frame, &sample) in piece.chunks_mut(channels).zip(mono.iter()) {
            let sample = narrowed(sample);
            loudest = loudest.max(sample.unsigned_abs());
            frame.fill(sample);
        }
    }
    loudest
}
