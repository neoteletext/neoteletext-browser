//! The sound helper's loop: the trusted core's messages read on a thread
//! of their own and applied to the mixers, and the channels sounding told
//! back, the device running only while one does.

use std::collections::BTreeSet;
use std::io::{self, BufReader, BufWriter};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

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
                Ok(Some(message)) => {
                    // RULE-66's hops: when the helper read a LocalMidi.
                    #[cfg(feature = "timed-test")]
                    if let ToSound::Audio { audio: protocol::audio::Audio::LocalMidi { status, data1, .. }, .. } = &message {
                        protocol::timed_test::mark("sound", "local-midi-read", Instant::now(), format_args!("{status:02X} {data1}"));
                    }
                    Wake::Core(Some(message))
                }
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
    // The device's buffer (page-rules.md RULE-97): what it grants, and the
    // policy of when it is small.
    let (offered, rate_hz) = speaker.offered();
    let small = audio::buffer::small(offered, rate_hz);
    // The timed test's setting "the device buffer left at 480 frames", for
    // its figures before RULE-97 (RULE-66).
    #[cfg(feature = "timed-test")]
    let small = small.filter(|_| !protocol::timed_test::setting(protocol::timed_test::USUAL_DEVICE_BUFFER));
    let mut policy = audio::buffer::Policy::new(small.is_some());
    let mut open_locals = 0_usize;
    loop {
        // The one deadline the loop has is the linger's end, a ceiling on
        // the wait; with none it waits on its messages alone.
        let wake = match policy.due() {
            Some(due) => match woken.recv_timeout(due.saturating_duration_since(Instant::now())) {
                Ok(wake) => Some(wake),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            },
            None => match woken.recv() {
                Ok(wake) => Some(wake),
                Err(_) => return Ok(()),
            },
        };
        let change = match &wake {
            None => policy.wake(Instant::now()),
            Some(Wake::Told(audio::Told::DeviceTrouble)) => policy.trouble(),
            Some(_) => audio::buffer::Change::None,
        };
        resize(&mut speaker, &mut output, change, small, trace)?;
        let Some(wake) = wake else { continue };
        if take(&mut speaker, &mut output, wake, trace)? {
            return Ok(());
        }
        let now_open = speaker.mixers.lock().unwrap_or_else(std::sync::PoisonError::into_inner).locals_open();
        if now_open != open_locals {
            open_locals = now_open;
            let change = policy.locals(now_open, Instant::now());
            resize(&mut speaker, &mut output, change, small, trace)?;
        }
    }
}

/// Opens the device again at the size `change` asks for (RULE-97), and on
/// a fallback reports fault 7 with count 0 once to each view with a local
/// sound open, whose sounds go on playing.
fn resize(speaker: &mut Speaker, output: &mut impl io::Write, change: audio::buffer::Change, small: Option<u32>, trace: bool) -> Result<(), String> {
    use audio::buffer::{Change, Size};
    let (offered, rate_hz) = speaker.offered();
    let size = match (change, small) {
        (Change::None, _) | (Change::Small, None) => return Ok(()),
        (Change::Small, Some(frames)) => Size::Frames(frames),
        (Change::Usual | Change::FallBack, _) => audio::buffer::usual(offered, rate_hz),
    };
    match speaker.reopen(size) {
        Err(error) => eprintln!("neoteletext-sound: the audio output's buffer: {error}"),
        Ok(()) if trace => eprintln!("neoteletext-sound: the output device's buffer is {:?}", speaker.size()),
        Ok(()) => {}
    }
    if change == Change::FallBack {
        let views = speaker.mixers.lock().unwrap_or_else(std::sync::PoisonError::into_inner).views_with_locals();
        for channel in views {
            tell(output, &FromSound::LocalOverrun { channel, count: 0 })?;
        }
    }
    Ok(())
}

/// Takes one wake of the loop; whether the helper is to end.
fn take(speaker: &mut Speaker, output: &mut impl io::Write, wake: Wake, trace: bool) -> Result<bool, String> {
    match wake {
        Wake::Core(Some(ToSound::Audio { channel, audio: operation })) => {
            if trace && !matches!(operation, protocol::audio::Audio::Samples { .. } | protocol::audio::Audio::StreamSamples { .. }) {
                eprintln!("neoteletext-sound: channel {channel}: {operation:?}");
            }
            audio::apply(&speaker.mixers, channel, operation);
        }
        Wake::Core(Some(ToSound::Settings { sound, volume })) => audio::set_sound(&speaker.mixers, sound, volume),
        Wake::Core(Some(ToSound::Close) | None) => return Ok(true),
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
            tell(output, &FromSound::Sounding(sounding.into_iter().collect()))?;
        }
        Wake::Told(audio::Told::Metered { channel, values, count }) => {
            let values = values.get(..count).unwrap_or(&[]).to_vec();
            tell(output, &FromSound::Metered { channel, values })?;
        }
        Wake::Told(audio::Told::Overrun { channel }) => {
            eprintln!("neoteletext-sound: channel {channel}: its local sounds overran their time and were stopped");
            tell(output, &FromSound::LocalOverrun { channel, count: 1 })?;
        }
        Wake::Told(audio::Told::Cue { channel, voice, number }) => {
            tell(output, &FromSound::CueHeard { channel, audio_channel: voice, number })?;
        }
        Wake::Told(audio::Told::Spent { channel, microseconds }) => tell(output, &FromSound::Spent { channel, microseconds })?,
        // Taken by the loop's buffer policy (RULE-97).
        Wake::Told(audio::Told::DeviceTrouble) => {}
    }
    Ok(false)
}

/// Writes one message to the trusted core, at once.
fn tell(output: &mut impl io::Write, message: &FromSound) -> Result<(), String> {
    write_from_sound(output, message).and_then(|()| output.flush().map_err(Into::into)).map_err(|error| format!("cannot write to the trusted core: {error}"))
}
