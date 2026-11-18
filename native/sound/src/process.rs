//! The sound helper's loop: the trusted core's messages read on a thread
//! of their own and applied to the mixers, and the channels sounding told
//! back, the device running only while one does.

use std::collections::BTreeSet;
use std::io::{self, BufReader, BufWriter};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use crate::audio::{self, Speaker};
use protocol::sound::{FromSound, ToSound, read_to_sound, write_from_sound};

/// How often the loudest sample is reported while tracing.
const TRACE_EVERY: Duration = Duration::from_millis(500);

/// What wakes the helper's loop.
enum Wake {
    /// A message from the trusted core, or the end of its pipe.
    Core(Option<ToSound>),
    /// The pipe broke, or a message was malformed.
    Broken(String),
    /// The channels sounding changed, or whether the device is wanted.
    Sounding(BTreeSet<u32>),
    /// The audio thread metered a view's watch list, or stopped a view's
    /// local sounds (page-rules.md RULE-33, RULE-62).
    Told(audio::Told),
}

/// Plays the voice operations the trusted core sends on standard input
/// through the default output device, and tells it on standard output
/// which channels sound, until it closes the pipe or says to stop.
///
/// # Errors
/// No output device, a malformed message, or a pipe that fails.
pub fn run() -> Result<(), String> {
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
    // What the audio thread metered and stopped, for the trusted core.
    if let Some(told) = speaker.told.take() {
        let to_loop = wakes.clone();
        std::thread::spawn(move || {
            for told in told {
                if to_loop.send(Wake::Told(told)).is_err() {
                    return;
                }
            }
        });
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
                // The device runs only while a channel sounds (decision 53)
                // or a local sound plays, heard or not (RULE-62); the trace
                // says when it starts and stops (TODO-1243).
                let mut mixers = speaker.mixers.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                let playing = mixers.device_wanted();
                if !playing {
                    // One report of zeros, then none while it is stopped
                    // (RULE-33 item 3).
                    mixers.report_silence();
                }
                drop(mixers);
                if speaker.set_playing(playing) && trace {
                    eprintln!("neoteletext-sound: the output device {}", if playing { "started" } else { "stopped" });
                }
                tell(&mut output, &FromSound::Sounding(sounding.into_iter().collect()))?;
            }
            Wake::Told(audio::Told::Metered { channel, values, count }) => {
                let values = values.get(..count).unwrap_or(&[]).to_vec();
                tell(&mut output, &FromSound::Metered { channel, values })?;
            }
            Wake::Told(audio::Told::Overrun { channel }) => {
                eprintln!("neoteletext-sound: channel {channel}: its local sounds overran their time and were stopped");
                tell(&mut output, &FromSound::LocalOverrun { channel, count: 1 })?;
            }
            Wake::Told(audio::Told::Cue { channel, voice, number }) => {
                tell(&mut output, &FromSound::CueHeard { channel, audio_channel: voice, number })?;
            }
            Wake::Told(audio::Told::Spent { channel, microseconds }) => tell(&mut output, &FromSound::Spent { channel, microseconds })?,
        }
    }
    Ok(())
}

/// Writes one message to the trusted core, at once.
fn tell(output: &mut impl io::Write, message: &FromSound) -> Result<(), String> {
    write_from_sound(output, message).and_then(|()| output.flush().map_err(Into::into)).map_err(|error| format!("cannot write to the trusted core: {error}"))
}
