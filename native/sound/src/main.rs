//! The sound helper (decision 55): it plays every channel's sound through
//! the system's output device, as the trusted core's checked voice
//! operations say, and tells the core which channels sound. It holds no
//! secrets, reaches no network, reads no file of the user's, and is the
//! only process that links the system's sound library: a Linux without
//! ALSA's library cannot start it, and the browser then runs silent
//! (requirement 16.6).
//!
//! A malformed message from the trusted core ends it.

// On Windows, a desktop program: no console window opens with it.
#![cfg_attr(windows, windows_subsystem = "windows")]
// No panicking constructs: this program reads input from outside the
// process. The same list is in each such crate; scripts/check.sh checks
// that they agree.
#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

use std::collections::BTreeSet;
use std::io::{self, BufReader, BufWriter, Write as _};
use std::process::ExitCode;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use protocol::sound::{FromSound, ToSound, read_to_sound, write_from_sound};
use sound::audio::{self, Speaker};

/// How often the loudest sample is reported while tracing.
const TRACE_EVERY: Duration = Duration::from_millis(500);

/// What wakes the helper's loop.
enum Wake {
    /// A message from the trusted core, or the end of its pipe.
    Core(Option<ToSound>),
    /// The pipe broke, or a message was malformed.
    Broken(String),
    /// The channels sounding changed.
    Sounding(BTreeSet<u32>),
}

fn run() -> Result<(), String> {
    let mut speaker = Speaker::open().map_err(|reason| format!("no sound: {reason}"))?;
    let trace = std::env::var_os("NEOTELETEXT_AUDIO_TRACE").is_some();
    let (wakes, woken) = mpsc::channel();
    // The trusted core's messages, read on a thread of their own.
    let from_core = wakes.clone();
    std::thread::spawn(move || {
        let mut input = BufReader::new(io::stdin().lock());
        loop {
            let wake = match read_to_sound(&mut input) {
                Ok(Some(message)) => Wake::Core(Some(message)),
                Ok(None) => Wake::Core(None),
                Err(error) => Wake::Broken(error.to_string()),
            };
            let last = !matches!(wake, Wake::Core(Some(_)));
            if from_core.send(wake).is_err() || last {
                return;
            }
        }
    });
    // Which channels sound, from the mixer's own wakes (decision 46).
    if let Some(sounding_wakes) = speaker.wakes.take() {
        let mixers = Arc::clone(&speaker.mixers);
        let to_loop = wakes.clone();
        std::thread::spawn(move || audio::watch(&mixers, &sounding_wakes, |sounding| to_loop.send(Wake::Sounding(sounding)).is_ok()));
    }
    if trace {
        let peak = Arc::clone(&speaker.peak);
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(TRACE_EVERY);
                let loudest = peak.swap(0, std::sync::atomic::Ordering::Relaxed);
                if loudest > 0 {
                    eprintln!("neoteletext-sound: audio peak {loudest}");
                }
            }
        });
    }
    drop(wakes);
    let mut output = BufWriter::new(io::stdout().lock());
    for wake in woken {
        match wake {
            Wake::Core(Some(ToSound::Audio { channel, audio: operation })) => {
                if trace && !matches!(operation, protocol::audio::Audio::Samples { .. } | protocol::audio::Audio::StreamSamples { .. }) {
                    eprintln!("neoteletext-sound: channel {channel}: {operation:?}");
                }
                audio::apply(&speaker.mixers, channel, operation);
            }
            Wake::Core(Some(ToSound::Settings { sound, volume })) => audio::set_sound(&speaker.mixers, sound, volume),
            Wake::Core(Some(ToSound::Close) | None) => return Ok(()),
            Wake::Broken(reason) => return Err(format!("bad message from the trusted core: {reason}")),
            Wake::Sounding(sounding) => {
                // The device runs only while a channel sounds.
                speaker.set_playing(!sounding.is_empty());
                write_from_sound(&mut output, &FromSound::Sounding(sounding.into_iter().collect()))
                    .and_then(|()| output.flush().map_err(Into::into))
                    .map_err(|error| format!("cannot write to the trusted core: {error}"))?;
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    // First, before any thread: the sound output and nothing else.
    match confine::apply(confine::Role::Sound, None) {
        Ok(report) => eprintln!("neoteletext-sound: confined: {report}"),
        Err(error) => {
            eprintln!("neoteletext-sound: {error}");
            return ExitCode::FAILURE;
        }
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neoteletext-sound: {error}");
            ExitCode::FAILURE
        }
    }
}
