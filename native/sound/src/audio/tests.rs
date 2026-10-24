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
        Audio::StreamSamples { sound: SOUND, pcm: vec![1; 8] },
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
    apply(&mixers, 7, Audio::Stream { sound: SOUND, rate_hz: 8_000 });
    assert_eq!(mixers.lock().unwrap().channels.keys().copied().collect::<Vec<_>>(), [6, 7]);
    let mut data = vec![0_i16; 400];
    let mut scratch = vec![0_i16; 400];
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
    let mut data = vec![0_i16; 400];
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
        let Operation::Stream { ring, .. } = Operation::prepare(Audio::Stream { sound: SOUND, rate_hz }) else { panic!("a live sound is prepared as one") };
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
    let leftover = mixers.apply(3, Operation::Stream { sound: SOUND, rate_hz: 8_000, ring: vec![0; wrong] });
    assert_eq!(leftover.buffer.map(|ring| ring.len()), Some(wrong));
    let start = Audio::Start { channel: 0, sound: SOUND, gain: 100, fade_ms: 5, looped: false, duck_percent: 100, after_current: false };
    let _ = mixers.apply(3, Operation::Other(start));
    let _ = mixers.apply(3, Operation::Other(Audio::StreamSamples { sound: SOUND, pcm: vec![6_000; 4_000] }));
    assert_eq!(mixers.sounding(), BTreeSet::new(), "no live sound to play");
}
