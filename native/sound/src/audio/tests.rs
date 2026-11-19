//! The mixers' tests: every operation applied to its channel's mixer, the
//! settings and the tabs' mutes, the watcher's wakes, and the rendering.

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
    let mut scratch = vec![0_i32; 128];
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
    let mut scratch = vec![0_i32; 2 * PIECE];
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
    apply(&mixers, 3, Audio::Stream { channel: 0, sound: SOUND, rate_hz: 8_000 });
    apply(&mixers, 3, Audio::Start { channel: 0, sound: SOUND, gain: 100, fade_ms: 5, looped: false, duck_percent: 100, after_current: false });
    apply(&mixers, 3, Audio::StreamSamples { channel: 0, sound: SOUND, pcm: vec![6_000; 4_000] });
    let mut data = vec![0_i16; 400];
    render(&mixers, &mut data, 1, &mut [0; 512]);
    assert!((5_800..=6_200).contains(&data[300]), "{}", data[300]);
}

/// A tab that starts sounding while another plays on is reported, so
/// its speaker shows: the owner's "audio is playing on
/// <https://app.neottx.com/audio/113> but there is no speaker icon on the
/// tab", with another tab already playing. The device was already
/// running, and the watcher was woken only when it was to start or stop.
#[test]
fn a_second_tab_sounding_is_reported_while_the_first_plays_on() {
    let (wake, wakes) = std::sync::mpsc::sync_channel(1);
    let mut mixers = Mixers::new(8_000);
    mixers.wake = Some(wake.clone());
    let mixers = Mutex::new(mixers);
    play(&mixers, 1, 8_000);
    assert!(wakes.try_recv().is_ok(), "the first tab's sound wakes the watcher");
    play(&mixers, 2, 8_000);
    assert!(wakes.try_recv().is_ok(), "the second tab's sound wakes the watcher too, though the device already runs");
    // The watcher, so woken, reports both tabs.
    wake.send(()).unwrap();
    let mut reported = Vec::new();
    watch(&mixers, &wakes, |sounding| {
        reported.push(sounding);
        false
    });
    assert_eq!(reported, [BTreeSet::from([1, 2])], "both tabs show their speakers");
}

/// The watcher is woken when a sound starts, when the last ends and
/// when the set of sounding tabs changes, never while the same tabs go
/// on sounding, and it reports the sounding channels each time they
/// change.
#[test]
fn the_watcher_is_woken_only_when_the_sounding_tabs_change() {
    let (wake, wakes) = std::sync::mpsc::sync_channel(1);
    let mut mixers = Mixers::new(8_000);
    mixers.wake = Some(wake);
    let mixers = Mutex::new(mixers);
    apply(&mixers, 1, Audio::Define { sound: SOUND, rate_hz: 8_000, samples: 4000 });
    assert!(wakes.try_recv().is_err(), "a defined sound does not sound");
    play(&mixers, 1, 8_000);
    assert!(wakes.try_recv().is_ok(), "the first sound wakes the watcher");
    play(&mixers, 2, 4_000);
    assert!(wakes.try_recv().is_ok(), "a second channel sounding changes what the tabs show");
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
    assert!(speaker.set_playing(true), "the device starts");
    assert!(!speaker.set_playing(true), "once");
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(speaker.mixers.lock().unwrap().sounding(), BTreeSet::from([1]), "still playing after 300 ms: the device is pulling samples");
    assert!(wakes.recv_timeout(std::time::Duration::from_millis(1500)).is_ok(), "the end wakes the watcher");
    assert_eq!(speaker.mixers.lock().unwrap().sounding(), BTreeSet::new(), "finished: the device consumed the whole second");
    assert!(speaker.set_playing(false), "the device stops");
}

/// Finding F-55: only a definition or a live sound makes a channel's
/// mixer; any other operation on a channel without one changes nothing,
/// and a mute given first still holds when the mixer is made.
#[test]
fn an_operation_on_an_unknown_channel_makes_no_mixer() {
    let mixers = Mutex::new(Mixers::new(8_000));
    for (channel, operation) in [
        Audio::Samples { sound: SOUND, first: 0, pcm: vec![1; 8] },
        Audio::Forget(SOUND),
        Audio::Start { channel: 0, sound: SOUND, gain: 100, fade_ms: 0, looped: true, duck_percent: 100, after_current: false },
        Audio::Stop { channel: 0, fade_ms: 0 },
        Audio::Gain { channel: 0, gain: 50, fade_ms: 0 },
        Audio::StreamSamples { channel: 0, sound: SOUND, pcm: vec![1; 8] },
        Audio::Mute(true),
    ]
    .into_iter()
    .enumerate()
    {
        apply(&mixers, u32::try_from(channel).unwrap(), operation);
    }
    assert_eq!(mixers.lock().unwrap().channels.keys().collect::<Vec<_>>(), Vec::<&u32>::new(), "no mixer made");
    // Channel 6 was muted before it had a mixer: it is silent once it plays.
    play(&mixers, 6, 8_000);
    apply(&mixers, 7, Audio::Stream { channel: 0, sound: SOUND, rate_hz: 8_000 });
    assert_eq!(mixers.lock().unwrap().channels.keys().copied().collect::<Vec<_>>(), [6, 7]);
    let mut data = vec![0_i16; 400];
    let mut scratch = vec![0_i32; 400];
    render(&mixers, &mut data, 1, &mut scratch);
    // After the master's ramp, as in the test of muted tabs above.
    assert!(data[300..].iter().all(|&sample| sample == 0), "the mute held: {:?}", &data[300..310]);
}

/// Finding F-55: a sound's buffer is allocated before the lock is taken
/// and the mixer keeps the one it is given. Given a buffer already
/// holding a level and no samples, the sound plays that level, which it
/// could not if the mixer had allocated a silent buffer of its own.
#[test]
fn a_sound_plays_the_buffer_allocated_before_the_lock() {
    let mut mixers = Mixers::new(8_000);
    let leftover = mixers.apply(1, Operation::Define { sound: SOUND, rate_hz: 8_000, samples: vec![8_000; 4_000] });
    assert!(leftover.mixer.is_none() && leftover.buffer.is_none(), "the buffer was taken");
    let start = Audio::Start { channel: 0, sound: SOUND, gain: 100, fade_ms: 5, looped: false, duck_percent: 100, after_current: false };
    let _ = mixers.apply(1, Operation::Other(start));
    let mut data = vec![0_i32; 400];
    mixers.render(&mut data, &mut [0; 400]);
    // After the 5 ms ramp, 40 samples at 8 kHz.
    assert!((7_900..=8_100).contains(&data[300]), "{}", data[300]);
}

/// The buffers prepared outside the lock: a definition's of its length,
/// a live sound's of the ring the mixer takes, and none for the others.
#[test]
fn a_definition_and_a_live_sound_are_given_their_buffers_before_the_lock() {
    let Operation::Define { samples, .. } = Operation::prepare(Audio::Define { sound: SOUND, rate_hz: 8_000, samples: 4_000 }) else {
        panic!("a definition is prepared as one")
    };
    assert_eq!(samples.len(), 4_000);
    // The rate the tests above play at, and the device's usual one.
    for rate_hz in [8_000, 48_000] {
        let Operation::Stream { ring, .. } = Operation::prepare(Audio::Stream { channel: 0, sound: SOUND, rate_hz }) else {
            panic!("a live sound is prepared as one")
        };
        assert_eq!(ring.len(), Mixer::stream_ring(rate_hz), "at {rate_hz} Hz");
    }
    assert!(matches!(Operation::prepare(Audio::Forget(SOUND)), Operation::Other(Audio::Forget(_))));
}

/// A ring the mixer refuses is handed back, to be freed after the lock
/// is let go, and no live sound is made.
#[test]
fn a_refused_ring_is_handed_back_to_be_freed_outside_the_lock() {
    let mut mixers = Mixers::new(8_000);
    let wrong = Mixer::stream_ring(8_000) + 1;
    let leftover = mixers.apply(3, Operation::Stream { channel: 0, sound: SOUND, rate_hz: 8_000, ring: vec![0; wrong] });
    assert_eq!(leftover.buffer.map(|ring| ring.len()), Some(wrong));
    let start = Audio::Start { channel: 0, sound: SOUND, gain: 100, fade_ms: 5, looped: false, duck_percent: 100, after_current: false };
    let _ = mixers.apply(3, Operation::Other(start));
    let _ = mixers.apply(3, Operation::Other(Audio::StreamSamples { channel: 0, sound: SOUND, pcm: vec![6_000; 4_000] }));
    assert_eq!(mixers.sounding(), BTreeSet::new(), "no live sound to play");
}

/// A start of `SOUND` on audio channel `voice` at full gain.
fn start_on(voice: u8, after_current: bool) -> Audio {
    Audio::Start { channel: voice, sound: SOUND, gain: 100, fade_ms: 5, looped: false, duck_percent: 100, after_current }
}

/// The engine's INT-37: a stream is named by its audio channel and its
/// sound, so one sound streaming on two audio channels is two voices, each
/// with its own samples, and ending one leaves the other playing.
#[test]
fn one_sound_on_two_audio_channels_plays_as_two_independent_voices() {
    let mixers = Mutex::new(Mixers::new(8_000));
    // Channel 0 holds the mixer's buffering target, 400 ms; channel 2
    // more than a second, short of what would make the mixer skip ahead.
    for (voice, level, samples) in [(0_u8, 6_000_i16, 3_200), (2, 2_000, 9_000)] {
        apply(&mixers, 3, Audio::Stream { channel: voice, sound: SOUND, rate_hz: 8_000 });
        apply(&mixers, 3, start_on(voice, false));
        apply(&mixers, 3, Audio::StreamSamples { channel: voice, sound: SOUND, pcm: vec![level; samples] });
    }
    let mut data = vec![0_i16; 400];
    render(&mixers, &mut data, 1, &mut [0; 512]);
    assert!((7_800..=8_200).contains(&data[300]), "both voices, each its own samples: {}", data[300]);
    // The stream on channel 0 ends; what it holds plays out, then only
    // channel 2's is heard.
    apply(&mixers, 3, Audio::EndStream { channel: 0, sound: SOUND });
    let mut data = vec![0_i16; 4_000];
    render(&mixers, &mut data, 1, &mut [0; 512]);
    assert!((7_800..=8_200).contains(&data[2_000]), "both, while channel 0's plays out: {}", data[2_000]);
    assert!((1_900..=2_100).contains(&data[3_500]), "channel 2's alone: {}", data[3_500]);
}

/// The engine's INT-36: a stream's end plays out what its voice holds, then
/// ends it, and the sound queued after it on that audio channel starts.
#[test]
fn a_stream_end_starts_the_queued_sound() {
    let mixers = Mutex::new(Mixers::new(8_000));
    let queued = Sound { handle: 5, generation: 1 };
    apply(&mixers, 1, Audio::Stream { channel: 0, sound: SOUND, rate_hz: 8_000 });
    apply(&mixers, 1, start_on(0, false));
    // Half a second of the stream: above the mixer's buffering target.
    apply(&mixers, 1, Audio::StreamSamples { channel: 0, sound: SOUND, pcm: vec![6_000; 4_000] });
    apply(&mixers, 1, Audio::Define { sound: queued, rate_hz: 8_000, samples: 8_000 });
    apply(&mixers, 1, Audio::Samples { sound: queued, first: 0, pcm: vec![2_000; 8_000] });
    apply(&mixers, 1, Audio::Start { channel: 0, sound: queued, gain: 100, fade_ms: 5, looped: false, duck_percent: 100, after_current: true });
    let mut data = vec![0_i16; 4_800];
    render(&mixers, &mut data, 1, &mut [0; 512]);
    assert!((5_800..=6_200).contains(&data[300]), "the stream: {}", data[300]);
    // Its 4 000 samples end a little after 4 000 out, past the ramp in.
    assert!(data[4_200..].iter().all(|&sample| sample == 0), "without its end, the starved stream waits and nothing follows");
    apply(&mixers, 1, Audio::EndStream { channel: 0, sound: SOUND });
    let mut data = vec![0_i16; 800];
    render(&mixers, &mut data, 1, &mut [0; 512]);
    assert!((1_900..=2_100).contains(&data[600]), "the queued sound: {}", data[600]);
}

/// A stream begun again under a name still held replaces it, and a forget
/// of a sound ends every stream of it, on every audio channel.
#[test]
fn a_stream_under_a_held_name_replaces_it_and_a_forget_ends_every_stream() {
    let mixers = Mutex::new(Mixers::new(8_000));
    apply(&mixers, 1, Audio::Stream { channel: 0, sound: SOUND, rate_hz: 8_000 });
    apply(&mixers, 1, Audio::StreamSamples { channel: 0, sound: SOUND, pcm: vec![6_000; 4_000] });
    apply(&mixers, 1, Audio::Stream { channel: 0, sound: SOUND, rate_hz: 8_000 });
    apply(&mixers, 1, start_on(0, false));
    apply(&mixers, 1, Audio::StreamSamples { channel: 0, sound: SOUND, pcm: vec![3_000; 4_000] });
    let mut data = vec![0_i16; 400];
    render(&mixers, &mut data, 1, &mut [0; 512]);
    assert!((2_900..=3_100).contains(&data[300]), "the new stream's samples: {}", data[300]);
    apply(&mixers, 1, Audio::Stream { channel: 1, sound: SOUND, rate_hz: 8_000 });
    apply(&mixers, 1, start_on(1, false));
    apply(&mixers, 1, Audio::StreamSamples { channel: 1, sound: SOUND, pcm: vec![1_000; 4_000] });
    apply(&mixers, 1, Audio::Forget(SOUND));
    let mut data = vec![0_i16; 8_000];
    render(&mixers, &mut data, 1, &mut [0; 512]);
    assert!(data[7_000..].iter().all(|&sample| sample == 0), "both streams played out and ended");
    assert_eq!(mixers.lock().unwrap().sounding(), BTreeSet::new());
}

/// A tenth of a second of a 440 Hz sine of `peak` at 48 kHz, which loops
/// without a step: 44 whole cycles.
fn sine(peak: f64) -> Vec<i16> {
    #[expect(clippy::cast_possible_truncation, reason = "a sine of at most a 16-bit peak, rounded, is a 16-bit sample")]
    (0..4_800_u32).map(|at| (peak * (f64::from(at) * 440.0 * std::f64::consts::TAU / 48_000.0).sin()).round() as i16).collect()
}

/// `tabs` channels at 48 kHz, each with all four voices looping a sine of
/// `peak` at full gain.
fn loud_tabs(tabs: u32, peak: f64) -> Mutex<Mixers> {
    let mixers = Mutex::new(Mixers::new(48_000));
    for tab in 1..=tabs {
        for voice in 0..4_u8 {
            let sound = Sound { handle: voice.wrapping_add(1), generation: 1 };
            apply(&mixers, tab, Audio::Define { sound, rate_hz: 48_000, samples: 4_800 });
            apply(&mixers, tab, Audio::Samples { sound, first: 0, pcm: sine(peak) });
            apply(&mixers, tab, Audio::Start { channel: voice, sound, gain: 100, fade_ms: 5, looped: true, duck_percent: 100, after_current: false });
        }
    }
    mixers
}

/// The longest run of samples next to each other that are equal and at
/// the loudest magnitude of `data`: a clamp leaves the wave's tops flat,
/// many samples long.
fn longest_flat_top(data: &[i16]) -> usize {
    let top = data.iter().map(|sample| sample.unsigned_abs()).max().unwrap_or(0);
    let (mut longest, mut run) = (0_usize, 0_usize);
    for sample in data {
        run = if sample.unsigned_abs() == top { run.wrapping_add(1) } else { 0 };
        longest = longest.max(run);
    }
    longest
}

/// REQ-6933 items 1 and 2 (TODO-6940, DEC-6950): four voices at full gain
/// in each of three tabs, a sum twelve times over full scale that the
/// clamp cut flat, come out at or under the ceiling of -1 dBFS, with no
/// sample at full scale and the wave's tops still round. One tab's four
/// voices alone are limited as well, which the engine mixer's clamp hid
/// from the helper before.
#[test]
fn a_sum_that_clipped_comes_out_under_the_ceiling_and_is_not_cut_flat() {
    for tabs in [3, 1] {
        let mixers = loud_tabs(tabs, 32_767.0);
        let mut data = vec![0_i16; 48_000];
        render(&mixers, &mut data, 1, &mut vec![0; 2 * 48_000]);
        let loudest = data.iter().map(|sample| sample.unsigned_abs()).max().unwrap();
        assert!(i32::from(loudest) <= 29_203, "{tabs} tabs: the loudest sample, {loudest}, is over the ceiling of -1 dBFS, 29 203");
        assert!(data.iter().all(|&sample| sample != i16::MAX && sample != i16::MIN), "{tabs} tabs: a sample is at full scale");
        // Settled, the tops are a sine's: a 440 Hz wave at 48 kHz holds its
        // top for at most two samples.
        let flat = longest_flat_top(&data[24_000..]);
        assert!(flat <= 2, "{tabs} tabs: the wave's top is flat for {flat} samples");
        // And it is loud: turned down to the ceiling, not below it.
        assert!(loudest >= 29_000, "{tabs} tabs: the loudest sample is only {loudest}");
    }
}

/// REQ-6933 item 4 (TODO-6940): what a device buffer of 10 milliseconds
/// costs to fill, printed, with three tabs of four voices each over the
/// ceiling and under it.
#[test]
fn the_cost_of_filling_a_device_buffer_is_printed() {
    const BLOCK: usize = 480;
    const BLOCKS: usize = 2_000;
    for (name, peak) in [("over the ceiling", 30_000.0), ("under the ceiling", 2_000.0)] {
        let mixers = loud_tabs(3, peak);
        let mut data = vec![0.0_f32; 2 * BLOCK];
        let mut scratch = [0; 2 * BLOCK];
        let mut costs = Vec::with_capacity(BLOCKS);
        for _ in 0..BLOCKS {
            let started = std::time::Instant::now();
            render(&mixers, &mut data, 2, &mut scratch);
            costs.push(started.elapsed());
        }
        costs.sort();
        println!("a 10 ms device buffer, 3 tabs of 4 voices {name}: median {:?}, slowest {:?}", costs[BLOCKS / 2], costs[BLOCKS - 1]);
    }
}

/// REQ-6933 item 3: the end of a sound is still in the limiter when its
/// mixer falls silent, so the device is wanted until it has gone out, and
/// nothing of one sound waits there to be heard at the start of the next.
#[test]
fn the_device_runs_on_until_the_limiter_has_let_the_last_sound_out() {
    let mixers = playing(8_000);
    let delay = mixers.lock().unwrap().delay_frames();
    apply(&mixers, 1, Audio::Reset);
    // The reset took the mixer; the limiter holds nothing yet heard.
    let mut data = vec![0_i16; 400];
    render(&mixers, &mut data, 1, &mut [0; 800]);
    assert!(data.iter().all(|&sample| sample == 0));
    assert!(!mixers.lock().unwrap().device_wanted());
    // A sound cut off after a buffer: its last frames are in the limiter.
    play(&mixers, 1, 8_000);
    render(&mixers, &mut data, 1, &mut [0; 800]);
    apply(&mixers, 1, Audio::Reset);
    assert!(mixers.lock().unwrap().device_wanted(), "the limiter holds the end of the sound");
    let mut tail = vec![0_i16; delay];
    render(&mixers, &mut tail, 1, &mut [0; 800]);
    assert!(tail.iter().all(|&sample| sample == 8_000), "the frames held back go out: {tail:?}");
    assert!(!mixers.lock().unwrap().device_wanted(), "and then the device may stop");
}
