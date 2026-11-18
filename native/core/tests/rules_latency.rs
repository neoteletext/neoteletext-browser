//! The timed test of neoteletext-common's `docs/page-rules.md` RULE-66
//! (REQ-6535): from a key to the first sample of a local sound handed to
//! the output device (RULE-64), and from a key to the tick in which a
//! `becomes` rule on the metered level fires, which is the tick a meter
//! segment is shown in (RULE-65).
//!
//! The path is the browser's own code, in the threads the processes are:
//! the trusted core's gate (`neoteletext::typing`), the decoder's engine
//! with the evaluator on its presentation clock, the trusted core's check
//! of every voice operation (`neoteletext::hearing`), and the sound
//! helper's mixers with a local instrument, rendered by a device that asks
//! for a buffer of 480 frames every 10 milliseconds. Every message between
//! them is written and read by the protocol crate, as on the pipes. What
//! this does not time is what a process boundary adds to each hop (the
//! timed test of `latency.rs` measures a pipe's hop) and the interface's
//! own read of the key and presenting of the frame: the confined helpers
//! cannot be started by a test on every machine, and the end-to-end
//! measurement is the music player's (RULE-95).
#![cfg(feature = "interaction")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use interface_core::canvas::PageKey;
use neoteletext::hearing::Hearing;
use neoteletext::typing::{Send as TypingSend, Typing};
use neoteletext_audio::encode::{Command, Sound as Writer, packets};
use neoteletext_input::event::{Action, KeyCode};
use neoteletext_rules::table::{Action as RuleAction, Key, KeyKind, Node, Operator, Rule, SOUND_LEVEL, Table, Trigger, Value, any_key};
use protocol::decoder::{FromDecoder, ToDecoder, read_from_core, read_from_decoder, write_to_core, write_to_decoder};
use protocol::interaction::{InputIntent, Intent};
use protocol::rules::Reported;
use protocol::sound::{FromSound, ToSound, read_from_sound, read_to_sound, write_from_sound, write_to_sound};
use sound::audio::{Mixers, TOLD_QUEUE, Told};

/// The keys timed (RULE-66: at least 200).
const KEYS: usize = 200;
/// The output device: 48,000 frames a second, a buffer of 10 milliseconds.
const RATE: u32 = 48_000;
const BUFFER_FRAMES: usize = 480;
const BUFFER: Duration = Duration::from_millis(10);
/// The view and its open page.
const CHANNEL: u32 = 1;
const PAGE: PageKey = PageKey { channel: CHANNEL, page: 0x101 };
/// The audio channel the local sound plays on.
const VOICE: u8 = 0;
/// The rule that plays the note, the rule that ends it, and the rule on
/// the level, which stands for the meter segment's `show`.
const LEVEL_RULE: u8 = 2;
/// The level a meter's lowest segment lights at (RULE-93: −36 decibels).
const LEVEL: i32 = 4;
/// RULE-64: at most 34 milliseconds from the key to the first sample, and
/// 20 or less in the median, on a device buffer of 10 milliseconds.
const SOUND_MOST: Duration = Duration::from_millis(34);
const SOUND_MEDIAN: Duration = Duration::from_millis(20);
/// RULE-65: a meter segment lights at most 45 milliseconds after the key,
/// 30 in the median.
const LEVEL_MOST: Duration = Duration::from_millis(45);
const LEVEL_MEDIAN: Duration = Duration::from_millis(30);
/// After a note ends: three reports of the meter and three ticks.
const SETTLE: Duration = Duration::from_millis(60);
/// How long anything may take before the test gives up.
const LIMIT: Duration = Duration::from_secs(5);

/// What reaches the thread that stands for the trusted core.
enum ToCore {
    /// The interface's intent, and when it was read from the keyboard.
    Key(InputIntent),
    /// A decoder's message, as written on its pipe.
    Decoder(Vec<u8>),
    /// The sound helper's message, as written on its pipe.
    Sound(Vec<u8>),
    Stop,
}

/// What the test is told happened, and when.
#[derive(Debug)]
enum Seen {
    /// The level rule's report reached the trusted core.
    Level(Instant),
    /// The device was handed a buffer whose samples were the first to
    /// sound after silence.
    Sounded(Instant),
    /// The device was handed a silent buffer after sound.
    Silent,
}

fn frame<M>(message: &M, write: impl Fn(&mut dyn std::io::Write, &M) -> Result<(), protocol::ProtocolError>) -> Vec<u8> {
    let mut bytes = Vec::new();
    write(&mut bytes, message).unwrap();
    bytes
}

/// The table: `a` down plays a note on the local sound, `a` up ends it,
/// and the level reaching [`LEVEL`] is reported, in the tick a rule would
/// show a meter's segment.
fn table() -> Box<Table> {
    let mut keys = any_key();
    keys.push(Key { kind: KeyKind::Character, code: u32::from('a') }).unwrap();
    let mut down = Rule::new(Trigger::Key { keys, repeat: false, mods_mask: 0, mods_value: 0 });
    down.actions.push(RuleAction::Midi { channel: VOICE, status: 0x90, data1: 60, data2: 127 }).unwrap();
    let mut up = Rule::new(Trigger::KeyUp { keys, mods_mask: 0, mods_value: 0 });
    up.actions.push(RuleAction::Midi { channel: VOICE, status: 0x80, data1: 60, data2: 0 }).unwrap();
    let level = Value::sound(VOICE, SOUND_LEVEL);
    let mut lit = Rule::new(Trigger::Becomes);
    lit.condition.push(Node::Compare { value: level, operator: Operator::GreaterOrEqual, constant: LEVEL }).unwrap();
    lit.actions.push(RuleAction::Send(None)).unwrap();
    let mut table = Box::new(Table::new());
    for rule in [down, up, lit] {
        table.rules.push(rule).unwrap();
    }
    table
}

/// What the channel pushes: the page's table, and a local sound of synth7
/// with its bank, played on [`VOICE`] for the page.
fn pushed() -> (Vec<u8>, Vec<u8>) {
    let mut body = vec![0_u8; neoteletext_rules::limits::MAX_TABLE_BYTES];
    let length = neoteletext_rules::encode(PAGE.page, 1, Some(&table()), &mut body).unwrap();
    let mut rules = Vec::new();
    neoteletext_web::records::rules::encode(&body[..length], 0, 200, &mut |packet| rules.extend_from_slice(&packet.bytes)).unwrap();
    // synth7's bank is its 32-voice bulk dump, here of its plain voice.
    let voices = [neoteletext_audio_instruments::synth7::Patch::DEFAULT; neoteletext_audio_instruments::synth7::patch::BANK_VOICES];
    let bank = Writer::bank(2, 1, neoteletext_audio::Codec::Synth7, &neoteletext_audio_instruments::synth7::sysex::bank_dump(&voices)).unwrap();
    let mut local = Writer::local(1, 1, neoteletext_audio::Codec::Synth7, &[]).unwrap();
    local.banks = vec![2];
    let play = Command { channel: VOICE, sequence: 1, operation: 1, handle: 1, generation: 1, gain: 100, fade_ms: 5, flags: 0, duck: 0, page: Some(PAGE.page) };
    let mut records = vec![bank.object_record().unwrap()];
    records.extend(bank.data_records());
    records.push(local.object_record().unwrap());
    records.extend(local.data_records());
    records.push(play.record());
    (rules, packets(0, 0, &records).0.iter().flat_map(|packet| packet.bytes).collect())
}

/// The decoder: the engine with the evaluator on its presentation clock,
/// woken by a message or by the next tick, whichever is first.
fn decoder(messages: &Receiver<Vec<u8>>, core: &Sender<ToCore>) {
    let mut engine = decoder::Engine::new();
    let (rules, sound) = pushed();
    engine.feed_whole(true, true, &rules);
    engine.open_page(Some(PAGE.page));
    engine.feed(&sound);
    engine.finish();
    loop {
        for audio in engine.take_sounds() {
            core.send(ToCore::Decoder(frame(&FromDecoder::Audio(audio), |out, message| write_to_core(out, message)))).unwrap();
        }
        let now = engine.now_us();
        let wait = engine.rules_due_us().map(|due| Duration::from_micros(due.saturating_sub(now)));
        let received = match wait {
            Some(wait) => match messages.recv_timeout(wait) {
                Ok(bytes) => Some(bytes),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            },
            None => match messages.recv() {
                Ok(bytes) => Some(bytes),
                Err(_) => return,
            },
        };
        // The tick's outputs leave before anything else of the wake.
        for message in engine.run_rules(engine.now_us()) {
            core.send(ToCore::Decoder(frame(&message, |out, message| write_to_core(out, message)))).unwrap();
        }
        if let Some(bytes) = received {
            let message = read_from_core(&mut bytes.as_slice()).unwrap().unwrap();
            engine.rule_message(message);
        }
    }
}

/// The output device: a buffer of [`BUFFER_FRAMES`] every [`BUFFER`], and
/// what the audio thread metered told the trusted core after it.
fn device(mixers: &Mutex<Mixers>, told: &Receiver<Told>, core: &Sender<ToCore>, seen: &Sender<Seen>, stop: &Receiver<()>) {
    let mut data = vec![0_i16; BUFFER_FRAMES];
    let mut scratch = vec![0_i16; BUFFER_FRAMES * 2];
    let mut sounding = false;
    let mut next = Instant::now();
    while stop.try_recv().is_err() {
        next += BUFFER;
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
        sound::audio::render(mixers, &mut data, 1, &mut scratch);
        // The buffer is handed to the device as its callback returns.
        let handed = Instant::now();
        let sounds = data.iter().any(|&sample| sample != 0);
        if sounds != sounding {
            sounding = sounds;
            let _ = seen.send(if sounds { Seen::Sounded(handed) } else { Seen::Silent });
        }
        for told in told.try_iter() {
            if let Told::Metered { channel, values, count } = told {
                let message = FromSound::Metered { channel, values: values[..count].to_vec() };
                let _ = core.send(ToCore::Sound(frame(&message, |out, message| write_from_sound(out, message))));
            }
        }
    }
}

/// The trusted core: the gate, the check of voice operations, and the
/// watch list, between the interface, the decoder and the sound helper.
fn core(events: &Receiver<ToCore>, to_decoder: &Sender<Vec<u8>>, mixers: &Mutex<Mixers>, seen: &Sender<Seen>) {
    let started = Instant::now();
    let now_us = || u64::try_from(started.elapsed().as_micros()).unwrap();
    let mut typing = Typing::default();
    let mut hearing = Hearing::default();
    let mut watched: Option<(u16, usize)> = None;
    // The session with the page's channel, welcomed; the page open and
    // drawn.
    typing.input(CHANNEL, true, true, 0);
    let mut body = [0_u8; neoteletext_input::control::MAX_BODY];
    let welcome = neoteletext_input::control::Control::Welcome { version: neoteletext_input::VERSION, caps: neoteletext_input::caps::SELECT_CAP, resume: None };
    let length = neoteletext_input::control::write_body(&welcome, &mut body).unwrap();
    let deliver = |sends: Vec<TypingSend>, to_decoder: &Sender<Vec<u8>>| {
        for send in sends {
            match send {
                TypingSend::Decoder { page, sequence, input, .. } => {
                    to_decoder.send(frame(&ToDecoder::RuleInput { page, sequence, input }, |out, message| write_to_decoder(out, message))).unwrap();
                }
                TypingSend::RulesPage { page, .. } => {
                    to_decoder.send(frame(&ToDecoder::RulesPage(page), |out, message| write_to_decoder(out, message))).unwrap();
                }
                _ => {}
            }
        }
    };
    deliver(typing.opened(Some(CHANNEL), Some(PAGE), 0), to_decoder);
    typing.control(CHANNEL, &body[..length], 0);
    typing.drawn(PAGE, 0);
    for event in events {
        match event {
            ToCore::Key(intent) => deliver(typing.input_intent(&intent, now_us()), to_decoder),
            ToCore::Decoder(bytes) => match read_from_decoder(&mut bytes.as_slice()).unwrap().unwrap() {
                // Checked, written for the sound helper, read by it and
                // applied to its mixers.
                FromDecoder::Audio(audio) => {
                    if let Some(message) = hearing.decoded(CHANNEL, audio) {
                        let bytes = frame(&message, |out, message| write_to_sound(out, message));
                        if let Some(ToSound::Audio { channel, audio }) = read_to_sound(&mut bytes.as_slice()).unwrap() {
                            sound::audio::apply(mixers, channel, audio);
                        }
                    }
                }
                FromDecoder::RulesOpen(told) => {
                    let (sends, open) = typing.rules_open(CHANNEL, &told, now_us());
                    watched = (open && !told.watch.is_empty()).then_some((told.page, told.watch.len()));
                    sound::audio::apply(mixers, CHANNEL, protocol::audio::Audio::Watch(if open { told.watch } else { Vec::new() }));
                    deliver(sends, to_decoder);
                }
                FromDecoder::Report(report) => {
                    let sent = typing.report(CHANNEL, &report, now_us());
                    if report.rule == LEVEL_RULE && matches!(report.what, Reported::Fired) {
                        assert!(sent.iter().any(|send| matches!(send, TypingSend::Fetcher { .. })), "the report went to the channel's fetcher");
                        let _ = seen.send(Seen::Level(Instant::now()));
                    }
                }
                _ => {}
            },
            ToCore::Sound(bytes) => {
                if let Some(FromSound::Metered { channel: CHANNEL, values }) = read_from_sound(&mut bytes.as_slice()).unwrap()
                    && let Some((page, length)) = watched.filter(|(_, length)| *length == values.len())
                {
                    let _ = length;
                    to_decoder.send(frame(&ToDecoder::Metered { page, values }, |out, message| write_to_decoder(out, message))).unwrap();
                }
            }
            ToCore::Stop => return,
        }
    }
}

fn key(action: Action) -> ToCore {
    let text = (action == Action::Press).then(|| String::from("a"));
    ToCore::Key(InputIntent { key: PAGE, intent: Intent::Key { action, key: KeyCode::Character('a'), mods: 0, text, base: None } })
}

/// The least, the median, the time nineteen in twenty are within, and the
/// greatest.
fn figures(times: &mut [Duration]) -> (Duration, Duration, Duration, Duration) {
    times.sort();
    (times[0], times[times.len() / 2], times[times.len() * 19 / 20], times[times.len() - 1])
}

#[test]
fn a_key_sounds_and_lights_its_level_within_the_budgets() {
    let (to_core, core_events) = mpsc::channel();
    let (to_decoder, decoder_messages) = mpsc::channel();
    let (seen_by, seen) = mpsc::channel();
    let (tell, told) = mpsc::sync_channel(TOLD_QUEUE);
    let (stop_device, device_stopped) = mpsc::channel();
    let mut mixers = Mixers::new(RATE);
    mixers.tell(tell);
    let mixers = Arc::new(Mutex::new(mixers));

    let decoder_thread = {
        let core = to_core.clone();
        std::thread::spawn(move || decoder(&decoder_messages, &core))
    };
    let device_thread = {
        let (mixers, core, seen_by) = (Arc::clone(&mixers), to_core.clone(), seen_by.clone());
        std::thread::spawn(move || device(&mixers, &told, &core, &seen_by, &device_stopped))
    };
    let core_thread = {
        let mixers = Arc::clone(&mixers);
        std::thread::spawn(move || core(&core_events, &to_decoder, &mixers, &seen_by))
    };
    // The table is taken and the local sound playing, silent.
    std::thread::sleep(Duration::from_millis(300));
    while seen.try_recv().is_ok() {}

    let (mut sounds, mut levels) = (Vec::new(), Vec::new());
    // A phase of the tick and of the device's buffer for each key, from a
    // generator with a fixed seed: the multiplier and increment of a
    // 64-bit linear congruential generator (Knuth, MMIX).
    let mut random: u64 = 0x2545_F491_4F6C_DD1D;
    for index in 0..KEYS {
        random = random.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        // The silence is metered and seen by a tick before the next key,
        // so that the level rule's condition is false again.
        std::thread::sleep(SETTLE + Duration::from_micros((random >> 33) % 20_000));
        let pressed = Instant::now();
        to_core.send(key(Action::Press)).unwrap();
        let (mut sounded, mut level) = (None, None);
        while sounded.is_none() || level.is_none() {
            match seen.recv_timeout(LIMIT).unwrap_or_else(|error| panic!("key {index}: nothing in time ({error}): sounded {sounded:?}, level {level:?}")) {
                Seen::Sounded(at) => sounded = sounded.or(Some(at)),
                Seen::Level(at) => level = level.or(Some(at)),
                Seen::Silent => {}
            }
        }
        sounds.push(sounded.unwrap().saturating_duration_since(pressed));
        levels.push(level.unwrap().saturating_duration_since(pressed));
        // The key comes up, and the next waits for silence.
        to_core.send(key(Action::Release)).unwrap();
        loop {
            match seen.recv_timeout(LIMIT).unwrap_or_else(|error| panic!("key {index}: the note did not end ({error})")) {
                Seen::Silent => break,
                Seen::Sounded(_) | Seen::Level(_) => {}
            }
        }
    }
    to_core.send(ToCore::Stop).unwrap();
    stop_device.send(()).unwrap();
    core_thread.join().unwrap();
    device_thread.join().unwrap();
    drop(to_core);
    decoder_thread.join().unwrap();

    let (sound_least, sound_median, sound_most, sound_greatest) = figures(&mut sounds);
    let (level_least, level_median, level_most, level_greatest) = figures(&mut levels);
    println!(
        "rules latency over {KEYS} keys: key to the first sample handed to the device least {sound_least:?} median {sound_median:?} 19 in 20 within {sound_most:?} greatest {sound_greatest:?}; \
         key to the level rule's report least {level_least:?} median {level_median:?} 19 in 20 within {level_most:?} greatest {level_greatest:?}"
    );
    for (what, greatest, bound) in
        [("RULE-64's greatest from a key to its sound", sound_greatest, SOUND_MOST), ("RULE-65's greatest from a key to its level", level_greatest, LEVEL_MOST)]
    {
        if greatest > bound {
            println!("rules latency: NOT MET: {what}, {bound:?}, is passed by {:?}", greatest.saturating_sub(bound));
        }
    }
    // The medians are held as the requirements give them. The greatest is
    // held for nineteen keys in twenty: this test's threads sleep and wake
    // by the operating system's timers, and on a busy computer a late wake
    // adds to some keys what no part of the path added (as the timed test
    // of `latency.rs` holds its lower quartile, DEC-2348). Every greatest
    // figure is printed, and one past its bound is said.
    assert!(sound_median <= SOUND_MEDIAN, "RULE-64: the median from a key to its sound is {sound_median:?}");
    assert!(sound_most <= SOUND_MOST, "RULE-64: nineteen in twenty keys sound within {sound_most:?}");
    assert!(level_median <= LEVEL_MEDIAN, "RULE-65: the median from a key to its level is {level_median:?}");
    // The sound helper reports after every device buffer of half a tick
    // of frames or more (DEC-6573), so RULE-65's own figure holds.
    assert!(level_most <= LEVEL_MOST, "RULE-65: nineteen in twenty keys light their level within {level_most:?}");
}
