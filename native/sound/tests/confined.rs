//! The sound helper, confined as the broker confines it, plays a tone
//! through the real output device and says when its channel starts and
//! stops sounding (decision 55). It needs an output device, so it runs only
//! when asked: `cargo test -- --ignored`.

use std::ffi::OsString;
use std::io::{BufReader, Write as _};
use std::path::Path;
use std::time::{Duration, Instant};

use confine::{Role, Spawn};
use protocol::audio::{Audio, Sound};
use protocol::sound::{FromSound, ToSound, read_from_sound, write_to_sound};

const SOUND: Sound = Sound { handle: 2, generation: 1 };
/// The tone's rate and length: one second at 16 kHz.
const RATE: u32 = 16_000;
/// How long the helper has to say each change.
const WAIT: Duration = Duration::from_secs(5);

/// Requirements 16.6 and 3.7: the sound helper, under its sandbox, opens
/// the device, plays, and reports its channel sounding and then silent.
#[test]
#[ignore = "needs an audio output device"]
fn a_tone_plays_through_the_confined_sound_helper() {
    let program = Path::new(env!("CARGO_BIN_EXE_neoteletext-sound"));
    let arguments: [OsString; 0] = [];
    let mut environment = confine::environment(Role::Sound);
    environment.push((OsString::from("NEOTELETEXT_AUDIO_TRACE"), OsString::from("1")));
    let mut process =
        confine::spawn(&Spawn { program, arguments: &arguments, role: Role::Sound, environment: &environment, local_network: false, writable: None }).unwrap();
    eprintln!("sound helper: {}", process.report);
    let mut input = process.stdin.take().unwrap();
    let mut output = BufReader::new(process.stdout.take().unwrap());
    // A triangle wave of about 889 Hz: 18 samples a period.
    let tone: Vec<i16> = (0..RATE)
        .map(|n| {
            let phase = i16::try_from(n % 18).unwrap();
            (if phase < 9 { phase } else { 18 - phase }) * 1_300 - 5_850
        })
        .collect();
    for message in [
        ToSound::Settings { sound: true, volume: 50 },
        ToSound::Audio { channel: 1, audio: Audio::Define { sound: SOUND, rate_hz: RATE, samples: RATE } },
        ToSound::Audio { channel: 1, audio: Audio::Samples { sound: SOUND, first: 0, pcm: tone } },
        ToSound::Audio {
            channel: 1,
            audio: Audio::Start { channel: 0, sound: SOUND, gain: 50, fade_ms: 20, looped: false, duck_percent: 100, after_current: false },
        },
    ] {
        write_to_sound(&mut input, &message).unwrap();
    }
    input.flush().unwrap();
    let started = Instant::now();
    assert_eq!(read_from_sound(&mut output).unwrap(), Some(FromSound::Sounding(vec![1])), "channel 1 sounds");
    assert!(started.elapsed() < WAIT);
    assert_eq!(read_from_sound(&mut output).unwrap(), Some(FromSound::Sounding(Vec::new())), "and stops when the second has played");
    let played = started.elapsed();
    assert!(played >= Duration::from_millis(800) && played < WAIT, "the device consumed the second in real time: {played:?}");
    write_to_sound(&mut input, &ToSound::Close).unwrap();
    input.flush().unwrap();
    assert!(process.wait().unwrap().success());
}
