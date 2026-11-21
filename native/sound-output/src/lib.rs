//! The sound helper's output stream (DEC-7168, replacing `cpal`): one
//! stream of one fixed format, signed 16-bit samples, two channels
//! interleaved, to the system's default output device, at the rate the
//! device runs at, with the buffer size the helper asks for (page-rules.md
//! RULE-97). It is all the sound helper needs, so each platform's part is
//! one path through that platform's own interface:
//!
//! - Linux: ALSA's PCM interface (`alsa/pcm.h`), the `default` device,
//!   which on a desktop is the sound server's plug-in, written from
//!   a thread of its own;
//! - macOS: Apple's default output audio unit (`AudioToolbox`,
//!   `kAudioUnitSubType_DefaultOutput`), which follows the system's chosen
//!   output device and calls back for each buffer;
//! - Windows: WASAPI in shared mode (`IAudioClient`), event driven, on a
//!   thread of its own.
//!
//! The stream is opened stopped; [`Output::play`] starts it and
//! [`Output::pause`] stops it, so that an idle browser costs the audio
//! thread nothing (decision 53). A device that fails while playing (an
//! underrun the platform reports, a device removed) is told through the
//! trouble callback, on whichever thread found it, and never ends the
//! process: the helper decides what to do (RULE-97 item 3).

#[cfg(target_os = "linux")]
mod alsa;
#[cfg(any(target_os = "linux", windows))]
mod control;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
use alsa as platform;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(windows)]
use windows as platform;

/// The channels of every stream: two, left and right interleaved. The
/// helper's mixers make one channel and copy it to each.
pub const CHANNELS: usize = 2;

/// The most frames a buffer may be asked for or granted: a second at
/// 192 kHz, the highest rate a stream is accepted at. A platform that
/// reports more is held to this, so a buffer is never allocated larger.
pub const MAX_BUFFER_FRAMES: u32 = MAX_RATE_HZ;

/// The lowest rate a device's stream is accepted at (the telephone rate),
/// and the highest: a device that reports a rate outside them is refused
/// rather than trusted with the mixers' sizes.
pub const MIN_RATE_HZ: u32 = 8_000;
/// See [`MIN_RATE_HZ`].
pub const MAX_RATE_HZ: u32 = 192_000;

/// What fills each buffer: interleaved frames of [`CHANNELS`] samples,
/// called on the audio thread. It must not block for long: the device
/// waits for it.
pub type Fill = Box<dyn FnMut(&mut [i16]) + Send>;

/// What is told of a failure of the device while it plays: an underrun,
/// a device that went away, a call that failed. Called on the audio thread
/// or the platform's own; it must not block.
pub type Trouble = Box<dyn FnMut(&str) + Send>;

/// The sizes the device grants for its buffer, in frames at its rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Granted {
    /// Any size from the first to the second.
    Range(u32, u32),
    /// Only its own size.
    Own,
}

/// The size a stream is opened with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// The device's own size.
    Own,
    /// This many frames at the device's rate.
    Frames(u32),
}

/// The default output device, as a stream would be opened on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Described {
    /// Its rate, which every stream on it runs at.
    pub rate_hz: u32,
    /// What it grants for its buffer.
    pub granted: Granted,
}

/// Why the device could not be described, opened, started or stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(String);

impl Error {
    fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// Describes the default output device: its rate and the buffer sizes it
/// grants.
///
/// # Errors
/// No output device, or one that cannot play the stream's format.
pub fn describe() -> Result<Described, Error> {
    let described = platform::describe()?;
    checked(described)
}

/// A device's description held to the bounds: a rate within
/// [`MIN_RATE_HZ`] and [`MAX_RATE_HZ`], and a range of sizes within
/// [`MAX_BUFFER_FRAMES`] with its least first.
fn checked(described: Described) -> Result<Described, Error> {
    if !(MIN_RATE_HZ..=MAX_RATE_HZ).contains(&described.rate_hz) {
        return Err(Error(format!("the output device's rate of {} Hz is outside what is played", described.rate_hz)));
    }
    let granted = match described.granted {
        Granted::Range(least, most) => {
            let least = least.clamp(1, MAX_BUFFER_FRAMES);
            let most = most.min(MAX_BUFFER_FRAMES);
            if most < least { Granted::Own } else { Granted::Range(least, most) }
        }
        Granted::Own => Granted::Own,
    };
    Ok(Described { rate_hz: described.rate_hz, granted })
}

/// A stream to the default output device. Dropped, it stops and closes.
pub struct Output {
    stream: platform::Stream,
}

impl Output {
    /// Opens a stream on the device `described`, stopped, with a buffer of
    /// `size`, filled by `fill`.
    ///
    /// # Errors
    /// The device is gone, or will not open at that rate or size.
    pub fn open(described: &Described, size: Size, fill: Fill, trouble: Trouble) -> Result<Self, Error> {
        let size = match size {
            Size::Frames(frames) if frames == 0 || frames > MAX_BUFFER_FRAMES => return Err(Error(format!("a buffer of {frames} frames"))),
            other => other,
        };
        Ok(Self { stream: platform::Stream::open(described, size, fill, trouble)? })
    }

    /// Starts the stream, or goes on.
    ///
    /// # Errors
    /// The device will not start.
    pub fn play(&mut self) -> Result<(), Error> {
        self.stream.play()
    }

    /// Stops the stream, or keeps it stopped.
    ///
    /// # Errors
    /// The device will not stop.
    pub fn pause(&mut self) -> Result<(), Error> {
        self.stream.pause()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A device's rate and sizes are held to the bounds before the mixers
    /// are sized by them.
    #[test]
    fn a_description_is_held_to_the_bounds() {
        assert!(checked(Described { rate_hz: 0, granted: Granted::Own }).is_err(), "no rate");
        assert!(checked(Described { rate_hz: MAX_RATE_HZ + 1, granted: Granted::Own }).is_err(), "too high a rate");
        assert_eq!(
            checked(Described { rate_hz: 48_000, granted: Granted::Range(0, u32::MAX) }),
            Ok(Described { rate_hz: 48_000, granted: Granted::Range(1, MAX_BUFFER_FRAMES) }),
            "a range from nothing to everything is held to one frame and a second"
        );
        assert_eq!(
            checked(Described { rate_hz: 44_100, granted: Granted::Range(600, 300) }),
            Ok(Described { rate_hz: 44_100, granted: Granted::Own }),
            "a range backwards grants only the device's own size"
        );
    }

    /// A buffer of no frames or of more than the bound is refused before
    /// the device is asked.
    #[test]
    fn an_impossible_buffer_is_refused() {
        let described = Described { rate_hz: 48_000, granted: Granted::Own };
        for frames in [0, MAX_BUFFER_FRAMES + 1] {
            let opened = Output::open(&described, Size::Frames(frames), Box::new(|_| {}), Box::new(|_| {}));
            assert!(opened.is_err(), "{frames} frames");
        }
    }

    /// The real default output plays silence in real time, at the size it
    /// was asked for and at its own: half a second of buffers takes about
    /// half a second, and stopped it takes none. It needs an output
    /// device, so it runs only when asked; it plays nothing anyone can
    /// hear.
    #[test]
    #[ignore = "needs an audio output device"]
    fn the_default_output_takes_samples_in_real_time() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let described = describe().unwrap();
        eprintln!("the default output: {described:?}");
        let sizes = match described.granted {
            Granted::Range(least, most) => vec![Size::Own, Size::Frames(480_u32.clamp(least, most)), Size::Frames(256_u32.clamp(least, most))],
            Granted::Own => vec![Size::Own],
        };
        for size in sizes {
            let (filled, longest) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
            let (counted, measured) = (Arc::clone(&filled), Arc::clone(&longest));
            let fill: Fill = Box::new(move |data: &mut [i16]| {
                data.fill(0);
                counted.fetch_add(data.len() / CHANNELS, Ordering::Relaxed);
                measured.fetch_max(data.len() / CHANNELS, Ordering::Relaxed);
            });
            let mut output = Output::open(&described, size, fill, Box::new(|error| eprintln!("trouble: {error}"))).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(100));
            assert_eq!(filled.load(Ordering::Relaxed), 0, "{size:?}: stopped, nothing is taken");
            output.play().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(500));
            output.pause().unwrap();
            let frames = filled.load(Ordering::Relaxed);
            let milliseconds = frames * 1_000 / usize::try_from(described.rate_hz).unwrap();
            eprintln!("{size:?}: {frames} frames, buffers of up to {} frames: {milliseconds} ms of sound in 500 ms", longest.load(Ordering::Relaxed));
            assert!((400..700).contains(&milliseconds), "{size:?}: the device took {milliseconds} ms of samples in half a second");
            std::thread::sleep(std::time::Duration::from_millis(100));
            let after = filled.load(Ordering::Relaxed);
            std::thread::sleep(std::time::Duration::from_millis(200));
            assert_eq!(filled.load(Ordering::Relaxed), after, "{size:?}: paused, nothing more is taken");
        }
    }
}
