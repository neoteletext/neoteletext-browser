//! Tests of local sounds and metering in the sound helper, with a scripted
//! device: the test renders the buffers the output device would ask for
//! (neoteletext-common's `docs/page-rules.md` RULE-32, RULE-33, RULE-60,
//! RULE-62; REQ-6535).

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, sync_channel};

use neoteletext_audio_mix::LOCAL_BLOCK_SAMPLES;
use protocol::audio::{Audio, LocalStart, Sound, WatchSlot};

use super::local::{LOCAL_MIDI_QUEUE, LOCAL_OVERRUN_BUFFERS, Locals};
use super::{Mixers, TOLD_QUEUE, Told, apply, render};

/// The output device's rate, the instruments' own.
const RATE: u32 = 48_000;
/// The usual device buffer: 10 milliseconds of frames (RULE-64).
const BUFFER: usize = 480;
/// The view the tests play in.
const VIEW: u32 = 7;
/// The audio channel the local sound plays on.
const VOICE: u8 = 0;
const SOUND: Sound = Sound { handle: 1, generation: 1 };

/// The families that have a local sound, by codec code and variant.
const FAMILIES: [(&str, u8, u8); 9] = [
    ("synth7", 13, 0),
    ("synth106", 14, 0),
    ("synth60", 14, 1),
    ("synthD", 15, 0),
    ("synth53", 16, 0),
    ("synth52", 16, 1),
    ("synth51", 16, 2),
    ("synth8", 17, 0),
    ("synth80", 18, 0),
];

fn mixers() -> (Mutex<Mixers>, Receiver<Told>) {
    let (tell, told) = sync_channel(TOLD_QUEUE);
    let mut mixers = Mixers::new(RATE);
    mixers.tell(tell);
    (Mutex::new(mixers), told)
}

/// Starts a local sound of `family` on [`VOICE`], with no bank: the
/// family's own default patch.
fn start(mixers: &Mutex<Mixers>, family: u8, variant: u8) {
    let start = LocalStart { sound: SOUND, family, variant, option: 0, split: 0, gain: 100, fade_ms: 5, duck_percent: 100, setup: Vec::new() };
    apply(mixers, VIEW, Audio::Local { channel: VOICE, start });
}

fn midi(mixers: &Mutex<Mixers>, status: u8, data1: u8, data2: u8) {
    apply(mixers, VIEW, Audio::LocalMidi { channel: VOICE, status, data1, data2 });
}

/// One device buffer of `frames` mono frames.
fn buffer(mixers: &Mutex<Mixers>, frames: usize) -> Vec<i16> {
    let mut data = vec![0_i16; frames];
    render(mixers, &mut data, 1, &mut vec![0; frames.saturating_mul(2)]);
    data
}

/// The metered reports told since the last look, newest last.
fn metered(told: &Receiver<Told>) -> Vec<Vec<u8>> {
    told.try_iter().filter_map(|told| if let Told::Metered { channel: VIEW, values, count } = told { Some(values[..count].to_vec()) } else { None }).collect()
}

/// RULE-62, RULE-64 (REQ-6535): a note on queued before a buffer is given
/// to the instrument before that buffer's first block, for every family:
/// no buffer is held ahead, and a message waits at most one block. The
/// note's first sample is then the instrument's own to make: within the
/// first block for the families that sound at once, and in the second for
/// those whose voice begins at its next block (the positions are printed).
#[test]
fn a_note_queued_before_a_buffer_sounds_within_its_first_blocks() {
    let mut late = Vec::new();
    for (name, family, variant) in FAMILIES {
        let (mixers, _told) = mixers();
        start(&mixers, family, variant);
        // The sound plays, silent: several buffers with no note.
        for _ in 0..4 {
            assert!(buffer(&mixers, BUFFER).iter().all(|&sample| sample == 0), "{name}: silent with no note");
        }
        midi(&mixers, 0x90, 60, 127);
        let sounded = buffer(&mixers, BUFFER);
        let first = sounded.iter().position(|&sample| sample != 0);
        println!("local sound, {name}: the note's first sample is sample {first:?} of the buffer it was queued before");
        assert!(first.is_some_and(|first| first < 2 * LOCAL_BLOCK_SAMPLES), "{name}: the first sample of the note is at {first:?} of the buffer");
        if first.is_some_and(|first| first >= LOCAL_BLOCK_SAMPLES) {
            late.push(name);
        }
    }
    // synth106's default patch is silent for its first block: its voice
    // begins at sample 128, where every other family sounds within 35.
    assert_eq!(late, ["synth106"], "the families whose first sample is in the second block");
}

/// RULE-60, RULE-62 (REQ-6535): while a local sound plays the output
/// device is kept running, heard or not, and the view's speaker mark
/// follows what is heard; a stop releases every note and the device may
/// stop again.
#[test]
fn a_local_sound_keeps_the_device_running_and_shows_only_when_heard() {
    let (mixers, _told) = mixers();
    start(&mixers, 14, 0);
    buffer(&mixers, BUFFER);
    {
        let mixers = mixers.lock().unwrap();
        assert!(mixers.device_wanted(), "the device runs, so a note never waits for it to start");
        assert!(mixers.sounding().is_empty(), "nothing is heard yet: no speaker mark");
    }
    midi(&mixers, 0x90, 60, 100);
    buffer(&mixers, BUFFER);
    assert_eq!(mixers.lock().unwrap().sounding().into_iter().collect::<Vec<_>>(), [VIEW]);
    apply(&mixers, VIEW, Audio::Stop { channel: VOICE, fade_ms: 5 });
    // The ramp out, then silence: the voice has ended.
    for _ in 0..4 {
        buffer(&mixers, BUFFER);
    }
    assert!(buffer(&mixers, BUFFER).iter().all(|&sample| sample == 0));
    let mixers = mixers.lock().unwrap();
    assert!(!mixers.device_wanted(), "stopped: the device may stop");
    assert!(mixers.sounding().is_empty());
}

/// RULE-32, RULE-33 item 3, RULE-65 (REQ-6535, DEC-6573): what a view's
/// watch list reads is reported once half a tick of frames was rendered,
/// so that a device buffer of 10 milliseconds is reported at its end, in the
/// list's order: the audio channel's level, and the local sound's
/// envelope stage, envelope level and voice count; nothing is reported
/// with no list, and nothing is metered for an audio channel no slot
/// watches.
#[test]
fn a_watch_list_is_metered_each_half_tick_and_only_what_it_names() {
    let (mixers, told) = mixers();
    start(&mixers, 14, 0);
    buffer(&mixers, 2 * BUFFER);
    assert_eq!(metered(&told), [] as [Vec<u8>; 0], "no list is in force: no report");
    let watch = vec![
        WatchSlot { channel: VOICE, kind: 1, part: 0 },
        WatchSlot { channel: VOICE, kind: 2, part: 0 },
        WatchSlot { channel: VOICE, kind: 3, part: 0 },
        WatchSlot { channel: VOICE, kind: 4, part: 0 },
        WatchSlot { channel: 1, kind: 4, part: 0 },
    ];
    apply(&mixers, VIEW, Audio::Watch(watch));
    {
        let mixers = mixers.lock().unwrap();
        let mixer = &mixers.channels.get(&VIEW).unwrap().mixer;
        assert!(mixer.is_metered(0), "the watched audio channel's peak is kept");
        assert!((1..4).all(|voice| !mixer.is_metered(voice)), "an unwatched channel costs no meter (RULE-32)");
    }
    // 400 frames are half a tick at 48,000: one report.
    buffer(&mixers, 200);
    assert_eq!(metered(&told), [] as [Vec<u8>; 0], "less than half a tick of frames");
    buffer(&mixers, 200);
    assert_eq!(metered(&told), [vec![0, 0, 0, 0, 0]], "nothing sounds: zeros, in the list's order");
    midi(&mixers, 0x90, 60, 127);
    // A device buffer of 10 milliseconds is reported at its own end.
    buffer(&mixers, BUFFER);
    let reports = metered(&told);
    let [report] = reports.as_slice() else { panic!("{reports:?}") };
    assert!(report[0] > 0, "the level of the buffer in which the note began: {report:?}");
    assert!((1..=3).contains(&report[1]), "attack, decay or sustain: {report:?}");
    assert!(report[2] > 0 && report[3] == 1, "the envelope's level, and one voice: {report:?}");
    assert_eq!(report[4], 0, "no local sound on audio channel 1");
    // The list ends: nothing more is reported.
    apply(&mixers, VIEW, Audio::Watch(Vec::new()));
    buffer(&mixers, 1_600);
    assert_eq!(metered(&told), [] as [Vec<u8>; 0]);
    assert!(!mixers.lock().unwrap().channels.get(&VIEW).unwrap().mixer.is_metered(0));
}

/// RULE-33 item 3 (REQ-6535): when the output device stops, one report of
/// zeros goes for each view with a list, and then none.
#[test]
fn a_stopped_device_reports_zeros_once() {
    let (mixers, told) = mixers();
    apply(&mixers, VIEW, Audio::Watch(vec![WatchSlot { channel: VOICE, kind: 1, part: 0 }, WatchSlot { channel: 2, kind: 1, part: 0 }]));
    mixers.lock().unwrap().report_silence();
    assert_eq!(metered(&told), [vec![0, 0]]);
    assert_eq!(metered(&told), [] as [Vec<u8>; 0]);
}

/// RULE-62 (REQ-6535): the queue holds `LOCAL_MIDI_QUEUE` messages; one
/// arriving at a full queue is dropped and counted, and the rest are
/// applied in arrival order before the next block.
#[test]
fn a_full_queue_drops_and_counts() {
    let (mixers, _told) = mixers();
    start(&mixers, 14, 0);
    // Note on and note off of one key, in turn: the queue ends on a note
    // off, so in arrival order nothing is left sounding.
    for index in 0..LOCAL_MIDI_QUEUE + 3 {
        midi(&mixers, if index % 2 == 0 { 0x90 } else { 0x80 }, 60, 100);
    }
    assert_eq!(mixers.lock().unwrap().channels.get(&VIEW).unwrap().locals.voices[0].as_ref().unwrap().dropped, 3);
    buffer(&mixers, BUFFER);
    for _ in 0..200 {
        buffer(&mixers, BUFFER);
    }
    assert!(buffer(&mixers, BUFFER).iter().all(|&sample| sample == 0), "the last message kept was a note off");
}

/// RULE-62 (REQ-6535): a view whose local sounds take longer to render
/// than the buffers they fill, for `LOCAL_OVERRUN_BUFFERS` buffers
/// running, is stopped; one buffer in time starts the count again, and a
/// view with no local sound is never counted.
#[test]
fn local_sounds_that_overrun_their_time_are_stopped() {
    let mut none = Locals::default();
    assert!((0..100).all(|_| !none.overran(20_000, 10_000)), "no local sound: nothing to stop");
    let (mixers, told) = mixers();
    start(&mixers, 14, 0);
    midi(&mixers, 0x90, 60, 100);
    buffer(&mixers, BUFFER);
    {
        let mut mixers = mixers.lock().unwrap();
        let locals = &mut mixers.channels.get_mut(&VIEW).unwrap().locals;
        for _ in 1..LOCAL_OVERRUN_BUFFERS {
            assert!(!locals.overran(10_001, 10_000));
        }
        assert!(!locals.overran(10_000, 10_000), "a buffer rendered in its time");
        for _ in 1..LOCAL_OVERRUN_BUFFERS {
            assert!(!locals.overran(10_001, 10_000), "the count began again");
        }
        assert!(locals.overran(10_001, 10_000), "the eighth buffer running");
    }
    assert!(told.try_iter().all(|told| !matches!(told, Told::Overrun { .. })), "rendered in time here: no overrun was told");
}

/// RULE-62 (REQ-6535): the measured cost of rendering one device buffer
/// of 480 frames with eight notes held, for each family, printed with the
/// time the buffer fills: the figure the requirement records with its
/// date. It fails only where a family cannot render a buffer in the time
/// it fills, in a build with optimisation.
#[test]
fn the_cost_of_a_buffer_of_eight_held_notes_is_measured() {
    const BUFFERS: u32 = 200;
    let fills_us = u128::try_from(BUFFER).unwrap() * 1_000_000 / u128::from(RATE);
    for (name, family, variant) in FAMILIES {
        let (mixers, _told) = mixers();
        start(&mixers, family, variant);
        for key in [48, 52, 55, 60, 64, 67, 72, 76] {
            midi(&mixers, 0x90, key, 100);
        }
        buffer(&mixers, BUFFER);
        let mut worst = 0;
        let started = std::time::Instant::now();
        for _ in 0..BUFFERS {
            let one = std::time::Instant::now();
            buffer(&mixers, BUFFER);
            worst = worst.max(one.elapsed().as_micros());
        }
        let mean = started.elapsed().as_micros() / u128::from(BUFFERS);
        println!("local sound cost, {name}: mean {mean} us, worst {worst} us, for a buffer of {BUFFER} frames that fills {fills_us} us");
        if !cfg!(debug_assertions) {
            assert!(mean < fills_us, "{name} renders a buffer in {mean} us, more than the {fills_us} us it fills");
        }
    }
}
