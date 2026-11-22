//! The script link's tests: every message round trips, each side refuses
//! the other's kinds, every bound holds the value at it and refuses the
//! value one past it whole, written or read, and a batch's sections come
//! at most once, in their order, each read to its exact length.

use neoteletext_rules::animate::{Operation, OperationKind};
use neoteletext_rules::limits::{
    AUDIO_CHANNELS, FIRST_FAMILY, LAST_FAMILY, LAST_SUBCODE, MAX_ANIMATE_OPERATIONS, MAX_GAIN, MAX_MIDI_DATA, MAX_PART, MAX_VARIANT,
};
use neoteletext_rules::table::{Target, TargetKind};

use super::batch::*;
use super::*;
use crate::framing::HEADER_BYTES;

/// A change to a message.
type Change<T> = Box<dyn Fn(&mut T)>;

pub(super) fn open() -> ContextOpen {
    ContextOpen {
        context: ContextId::Page,
        api: 1,
        renderer: Renderer::Regis,
        level: Level::TwoFive,
        page: 0x100,
        subcode: 0x0001,
        flags: ContextFlags { pointer_press: true, pointer_move: false, wheel: true, text: false, network: true, from_disk: false, sound_allowed: false },
        tick_divisor: 2,
        language: "en-GB".into(),
        channel_url: "https://a.example/news/".into(),
        page_url: "https://a.example/news/100".into(),
        script_url: "https://a.example/news/clock.js".into(),
        channel_title: "N\u{e9}ws".into(),
        seed: [9; SEED_BYTES],
        epoch_seconds: 1_795_000_000,
        zone_minutes: -330,
        keys: vec!["ArrowLeft".into(), "a".into()],
        sounds: vec![1, 2, 255],
        source_bytes: 1234,
    }
}

/// An opening with every field at its bound.
fn widest_open() -> ContextOpen {
    ContextOpen {
        subcode: LAST_SUBCODE,
        tick_divisor: MAX_TICK_DIVISOR,
        language: "a".repeat(MAX_LANGUAGE_BYTES),
        channel_url: "c".repeat(MAX_URL_BYTES),
        page_url: "p".repeat(MAX_URL_BYTES),
        script_url: "s".repeat(MAX_URL_BYTES),
        channel_title: "t".repeat(MAX_TITLE_BYTES),
        zone_minutes: MAX_ZONE_MINUTES,
        keys: vec!["k".repeat(MAX_KEY_NAME_BYTES); MAX_KEY_ASKS],
        sounds: vec![7; MAX_SOUND_HANDLES],
        source_bytes: u32::try_from(MAX_SCRIPT_BYTES).unwrap(),
        flags: ContextFlags { pointer_press: true, pointer_move: true, wheel: true, text: true, network: true, from_disk: true, sound_allowed: true },
        ..open()
    }
}

fn event() -> Event {
    Event { kind: EventKind::KeyDown, numbers: [1, -2, i32::MAX, i32::MIN], payload: b"ArrowLeft".to_vec() }
}

fn step() -> Step {
    Step {
        sequence: 41,
        tick: u64::MAX,
        missed: 3,
        flags: StepFlags { tick: true, pausing: false, resumed: true, closing: false, sound_allowed: true },
        events: EventKind::ALL.iter().map(|&kind| Event { kind, ..event() }).collect(),
        painted: vec![1, -1],
        cycle_ended: vec![5],
        bounds: vec![Bounds { handle: 3, x: -10, y: 20, width: 300, height: 4 }],
        sound: vec![SoundReport::Cue { voice: 1, cue: 9 }, SoundReport::Ended { voice: 3 }],
        faults: vec![FaultReport { code: 2, bound: "MAX_BATCH_BYTES".into() }],
    }
}

/// A step with every list and field at its bound.
fn widest_step() -> Step {
    Step {
        events: vec![Event { payload: vec![0xFF; MAX_EVENT_PAYLOAD_BYTES], ..event() }; MAX_STEP_EVENTS],
        painted: vec![1; MAX_WATCHED_PAINTINGS],
        cycle_ended: vec![1; MAX_CYCLE_ENDS],
        bounds: vec![Bounds { handle: 3, x: 0, y: 0, width: 1, height: 1 }; MAX_TRACKED_DRAWINGS],
        sound: vec![SoundReport::Cue { voice: 0, cue: 1 }; MAX_STEP_SOUND_REPORTS],
        faults: vec![FaultReport { code: 1, bound: "B".repeat(MAX_BOUND_NAME_BYTES) }; MAX_STEP_FAULTS],
        flags: StepFlags { tick: true, pausing: true, resumed: true, closing: true, sound_allowed: true },
        ..step()
    }
}

fn object(number: u16) -> Target {
    Target { kind: TargetKind::Object, number }
}

fn sprite(number: u16) -> Sprite {
    Sprite { number, shown: true, detectable: false, layer: 2, order: -3, column: -39, row: 24, width: 40, height: 25, objects: vec![1, MAX_OBJECT_NUMBER] }
}

fn naplps_object(number: u16) -> NaplpsObject {
    NaplpsObject {
        number,
        kind: NaplpsObjectKind::Macro,
        shown: true,
        detectable: true,
        layer: 1,
        order: 5,
        x: -4,
        y: 9,
        width: RASTER_WIDTH,
        height: RASTER_HEIGHT,
        origin: (1, -1),
        cover: 0,
        looks: [FIRST_MACRO, 0, LAST_MACRO],
        frames: vec![FIRST_MACRO, LAST_MACRO],
        label: "A b\u{e9}".into(),
    }
}

fn region(number: u16) -> NaplpsObject {
    NaplpsObject { kind: NaplpsObjectKind::Region, frames: Vec::new(), cover: 3, origin: (0, 0), looks: [0; LOOKS], ..naplps_object(number) }
}

/// Every operation a script may make.
fn operations() -> Vec<Operation> {
    vec![
        Operation { target: Target { kind: TargetKind::Control, number: 1 }, kind: OperationKind::Show },
        Operation { target: object(2), kind: OperationKind::Hide },
        Operation { target: object(2), kind: OperationKind::Look(3) },
        Operation { target: object(65_535), kind: OperationKind::Move { relative: true, x: -5, y: 7 } },
        Operation { target: object(2), kind: OperationKind::Order { layer: 3, order: -128 } },
        Operation { target: object(2), kind: OperationKind::Frame(15) },
        Operation { target: object(2), kind: OperationKind::Cycle { first: 0, last: 15, period: 3600, once: true } },
        Operation { target: object(2), kind: OperationKind::StopCycle },
    ]
}

fn sounds() -> Vec<SoundOperation> {
    vec![
        SoundOperation::Play { voice: 3, sound: 255, gain: MAX_GAIN },
        SoundOperation::Stop { voice: 0 },
        SoundOperation::SynthStart { part: MAX_PART, family: LAST_FAMILY, variant: MAX_VARIANT, gain: 1 },
        SoundOperation::Midi { part: 0, status: 0x90, data1: MAX_MIDI_DATA, data2: 0 },
        SoundOperation::SynthStop { part: 1 },
    ]
}

/// A batch with every section.
fn batch() -> Batch {
    Batch {
        sequence: 41,
        verdicts: Some(vec![Verdict { cancelled: true, handled: true }, Verdict::default(), Verdict { cancelled: false, handled: true }]),
        teletext_rows: Some(vec![TeletextRow { row: FIRST_SCRIPT_ROW, codes: [0x20; 40] }, TeletextRow { row: LAST_SCRIPT_ROW, codes: [MAX_ROW_CODE; 40] }]),
        teletext_enhancement: Some(vec![
            Triplet { address: MAX_TRIPLET_ADDRESS, mode: MAX_TRIPLET_MODE, data: MAX_TRIPLET_DATA },
            Triplet { address: 0, mode: 0, data: 0 },
        ]),
        teletext_settings: Some(TeletextSettings {
            screen_colour: LAST_COLOUR_ENTRY,
            row_colour: 0,
            substitution: true,
            colours: vec![(FIRST_DEFINABLE_ENTRY, MAX_RGB12), (LAST_COLOUR_ENTRY, 0)],
        }),
        teletext_objects: Some(vec![
            TeletextObject { number: 0, triplets: Vec::new() },
            TeletextObject { number: MAX_OBJECT_NUMBER, triplets: vec![Triplet { address: 1, mode: 2, data: 3 }] },
        ]),
        sprites: Some(vec![sprite(1), sprite(65_535)]),
        animate: Some(operations()),
        regis: Some(b"P[100,100]V[200,200]".to_vec()),
        naplps: Some(NaplpsAppend { new_generation: true, bytes: vec![0x0E, 0x21, 0xFF] }),
        naplps_objects: Some(NaplpsObjects { one: false, title: "A picture".into(), objects: vec![naplps_object(1), region(2)] }),
        sound: Some(sounds()),
        watch: Some(Watch { painted: Some(vec![1, 2]), tracked: None, wheel: Some(Vec::new()) }),
        labels: Some(vec![Label { handle: -7, label: "Play".into() }]),
        title: Some("A title".into()),
        navigate: Some("https://a.example/news/101".into()),
        log: Some(vec![LogLine { level: MAX_LOG_LEVEL, context: ContextId::Channel, text: "x".repeat(MAX_LOG_CHARS) }]),
        state: Some(vec![
            ContextState { context: ContextId::Channel, stopped: None, failures: 0, next_due: Some(u64::MAX) },
            ContextState { context: ContextId::Page, stopped: Some(FailReason::Memory), failures: 16, next_due: None },
        ]),
    }
}

fn to_host() -> Vec<ToScriptHost> {
    let mut every = vec![
        ToScriptHost::Hello { link_version: LINK_VERSION },
        ToScriptHost::ContextOpen(Box::new(open())),
        ToScriptHost::ContextOpen(Box::new(widest_open())),
        ToScriptHost::Source { context: ContextId::Page, offset: 0, bytes: vec![b'x'; MAX_SOURCE_PIECE_BYTES] },
        ToScriptHost::Source { context: ContextId::Channel, offset: u32::try_from(MAX_SCRIPT_BYTES - 1).unwrap(), bytes: vec![b';'] },
        ToScriptHost::Step(Box::new(step())),
        ToScriptHost::Step(Box::new(widest_step())),
        ToScriptHost::Step(Box::default()),
        ToScriptHost::ContextClose(ContextId::Page),
        ToScriptHost::ContextClose(ContextId::Channel),
        ToScriptHost::Shutdown,
    ];
    for &renderer in Renderer::ALL {
        for &level in Level::ALL {
            for &context in ContextId::ALL {
                every.push(ToScriptHost::ContextOpen(Box::new(ContextOpen { context, renderer, level, ..open() })));
            }
        }
    }
    every
}

fn to_decoder() -> Vec<FromScriptHost> {
    let mut every = vec![
        FromScriptHost::Ready { link_version: LINK_VERSION, api_max: 1 },
        FromScriptHost::Batch(Box::new(batch())),
        FromScriptHost::Batch(Box::new(Batch::new(u32::MAX))),
        FromScriptHost::Batch(Box::new(Batch { regis: Some(Vec::new()), title: Some(String::new()), sprites: Some(Vec::new()), ..Batch::new(1) })),
        FromScriptHost::Batch(Box::new(Batch {
            naplps_objects: Some(NaplpsObjects { one: true, title: String::new(), objects: vec![region(9)] }),
            ..Batch::new(2)
        })),
    ];
    for &reason in FailReason::ALL {
        every.push(FromScriptHost::Failed { context: ContextId::Page, reason, text: "\u{1F600}".repeat(MAX_LOG_CHARS) });
    }
    every
}

/// A frame of the given kind and payload.
fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![kind];
    frame.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

/// A text or byte field as the pipe carries it.
fn field(bytes: &[u8]) -> Vec<u8> {
    let mut field = u32::try_from(bytes.len()).unwrap().to_be_bytes().to_vec();
    field.extend_from_slice(bytes);
    field
}

/// A list as the pipe carries it: a count, which need not be the items',
/// and the items.
fn list(count: usize, items: &[u8]) -> Vec<u8> {
    let mut list = u16::try_from(count).unwrap().to_be_bytes().to_vec();
    list.extend_from_slice(items);
    list
}

/// The payload of `message` as it is written.
fn payload_to_host(message: &ToScriptHost) -> Vec<u8> {
    let mut pipe = Vec::new();
    write_to_host(&mut pipe, message).unwrap();
    pipe[HEADER_BYTES..].to_vec()
}

/// A batch's payload of the given sections, whatever they hold.
fn raw_batch(sections: &[(u8, Vec<u8>)]) -> Vec<u8> {
    let mut payload = 41_u32.to_be_bytes().to_vec();
    for (tag, body) in sections {
        payload.push(*tag);
        payload.extend_from_slice(&field(body));
    }
    payload
}

/// The sections of a batch as it is written: each tag with its body.
fn sections_of(batch: &Batch) -> Vec<(u8, Vec<u8>)> {
    let payload = batch.write().unwrap();
    let mut rest = &payload[4..];
    let mut sections = Vec::new();
    while !rest.is_empty() {
        let length = u32::from_be_bytes(rest[1..5].try_into().unwrap()) as usize;
        sections.push((rest[0], rest[5..5 + length].to_vec()));
        rest = &rest[5 + length..];
    }
    sections
}

/// The body of the one section of `batch`.
fn body_of(batch: &Batch) -> (u8, Vec<u8>) {
    let sections = sections_of(batch);
    assert_eq!(sections.len(), 1, "a batch of one section");
    sections.into_iter().next().unwrap()
}

fn read_batch(payload: &[u8]) -> Result<Option<FromScriptHost>, ProtocolError> {
    read_to_decoder(&mut frame(KIND_BATCH, payload).as_slice())
}

/// Whether `result` refused by exactly `what`.
fn refused<T: std::fmt::Debug>(result: &Result<T, ProtocolError>, what: &str) -> bool {
    matches!(result, Err(ProtocolError::BadValue(said)) if *said == what)
}

#[test]
fn every_message_round_trips() {
    let mut pipe = Vec::new();
    for message in to_host() {
        write_to_host(&mut pipe, &message).unwrap();
    }
    let mut input = pipe.as_slice();
    for message in to_host() {
        assert_eq!(read_to_host(&mut input).unwrap(), Some(message));
    }
    assert_eq!(read_to_host(&mut input).unwrap(), None);

    let mut pipe = Vec::new();
    for message in to_decoder() {
        write_to_decoder(&mut pipe, &message).unwrap();
    }
    let mut input = pipe.as_slice();
    for message in to_decoder() {
        assert_eq!(read_to_decoder(&mut input).unwrap(), Some(message));
    }
    assert_eq!(read_to_decoder(&mut input).unwrap(), None);
}

/// Each side reads only what the other may say, and the trusted core's
/// relay knows each kind's one direction.
#[test]
fn each_side_refuses_the_others_kinds() {
    for message in to_host() {
        let mut pipe = Vec::new();
        write_to_host(&mut pipe, &message).unwrap();
        assert!(is_to_host_kind(pipe[0]) && !is_to_decoder_kind(pipe[0]));
        assert!(matches!(read_to_decoder(&mut pipe.as_slice()), Err(ProtocolError::UnknownKind(_))), "{message:?}");
    }
    for message in to_decoder() {
        let mut pipe = Vec::new();
        write_to_decoder(&mut pipe, &message).unwrap();
        assert!(is_to_decoder_kind(pipe[0]) && !is_to_host_kind(pipe[0]));
        assert!(matches!(read_to_host(&mut pipe.as_slice()), Err(ProtocolError::UnknownKind(_))), "{message:?}");
    }
    for kind in 0..=u8::MAX {
        assert_eq!(KINDS.contains(&kind), is_to_host_kind(kind) || is_to_decoder_kind(kind), "kind {kind}");
    }
}

/// An opening one past any bound is neither written nor read.
#[test]
fn an_opening_one_past_a_bound_is_refused() {
    let wrong: Vec<(&str, Change<ContextOpen>)> = vec![
        ("page number", Box::new(|open| open.page = 0x0FF)),
        ("page number", Box::new(|open| open.page = 0x900)),
        ("sub-code", Box::new(|open| open.subcode = LAST_SUBCODE + 1)),
        ("tick divisor", Box::new(|open| open.tick_divisor = MAX_TICK_DIVISOR + 1)),
        ("language", Box::new(|open| open.language = "a".repeat(MAX_LANGUAGE_BYTES + 1))),
        ("language", Box::new(|open| open.language = "en GB".into())),
        ("channel address", Box::new(|open| open.channel_url = "c".repeat(MAX_URL_BYTES + 1))),
        ("channel address", Box::new(|open| open.channel_url = "https://a.example/ne ws/".into())),
        ("page address", Box::new(|open| open.page_url = "p".repeat(MAX_URL_BYTES + 1))),
        ("script address", Box::new(|open| open.script_url = "s".repeat(MAX_URL_BYTES + 1))),
        ("script address", Box::new(|open| open.script_url = "https://a.example/\n".into())),
        ("channel title", Box::new(|open| open.channel_title = "t".repeat(MAX_TITLE_BYTES + 1))),
        ("channel title", Box::new(|open| open.channel_title = "a\u{1b}[2J".into())),
        ("zone", Box::new(|open| open.zone_minutes = MAX_ZONE_MINUTES + 1)),
        ("zone", Box::new(|open| open.zone_minutes = -MAX_ZONE_MINUTES - 1)),
        ("too many keys", Box::new(|open| open.keys = vec!["k".into(); MAX_KEY_ASKS + 1])),
        ("key name", Box::new(|open| open.keys = vec!["k".repeat(MAX_KEY_NAME_BYTES + 1)])),
        ("key name", Box::new(|open| open.keys = vec![String::new()])),
        ("key name", Box::new(|open| open.keys = vec!["a\n".into()])),
        ("too many sounds", Box::new(|open| open.sounds = vec![1; MAX_SOUND_HANDLES + 1])),
        ("source length", Box::new(|open| open.source_bytes = 0)),
        ("source length", Box::new(|open| open.source_bytes = u32::try_from(MAX_SCRIPT_BYTES + 1).unwrap())),
    ];
    for (what, change) in wrong {
        let mut changed = open();
        change(&mut changed);
        assert!(refused(&changed.check(), what), "{what}: {:?}", changed.check());
        assert!(refused(&write_to_host(&mut Vec::new(), &ToScriptHost::ContextOpen(Box::new(changed))), what), "{what}");
    }
}

/// The parts of an opening's payload, written by hand so that each can
/// say what no writer would.
struct RawOpen {
    fixed: Vec<u8>,
    texts: [Vec<u8>; 5],
    zone: i16,
    keys: (usize, Vec<Vec<u8>>),
    sounds: (usize, Vec<u8>),
    source: u32,
}

impl RawOpen {
    fn new() -> Self {
        let open = open();
        Self {
            // The context, the API version, the renderer, the level, the
            // page, the sub-code, the flags and the tick divisor.
            fixed: vec![2, 0, 1, 2, 25, 0x01, 0x00, 0x00, 0x01, 0b01_0101, 2],
            texts: [open.language, open.channel_url, open.page_url, open.script_url, open.channel_title].map(String::into_bytes),
            zone: open.zone_minutes,
            keys: (open.keys.len(), open.keys.into_iter().map(String::into_bytes).collect()),
            sounds: (open.sounds.len(), open.sounds),
            source: open.source_bytes,
        }
    }

    fn payload(&self) -> Vec<u8> {
        let mut payload = self.fixed.clone();
        for text in &self.texts {
            payload.extend_from_slice(&field(text));
        }
        payload.extend_from_slice(&[9; SEED_BYTES]);
        payload.extend_from_slice(&1_795_000_000_u64.to_be_bytes());
        payload.extend_from_slice(&self.zone.to_be_bytes());
        payload.extend_from_slice(&list(self.keys.0, &self.keys.1.iter().flat_map(|key| field(key)).collect::<Vec<u8>>()));
        payload.extend_from_slice(&list(self.sounds.0, &self.sounds.1));
        payload.extend_from_slice(&self.source.to_be_bytes());
        payload
    }

    fn read(&self) -> Result<Option<ToScriptHost>, ProtocolError> {
        read_to_host(&mut frame(KIND_CONTEXT_OPEN, &self.payload()).as_slice())
    }
}

/// The same bounds, read from bytes no writer of this crate made.
#[test]
fn an_opening_one_past_a_bound_is_not_read() {
    assert_eq!(RawOpen::new().payload(), payload_to_host(&ToScriptHost::ContextOpen(Box::new(open()))), "the hand's layout is the writer's");
    assert_eq!(RawOpen::new().read().unwrap(), Some(ToScriptHost::ContextOpen(Box::new(open()))));
    let wrong: Vec<(&str, Change<RawOpen>)> = vec![
        ("context", Box::new(|raw| raw.fixed[0] = 3)),
        ("context", Box::new(|raw| raw.fixed[0] = 0)),
        ("renderer", Box::new(|raw| raw.fixed[3] = 4)),
        ("level", Box::new(|raw| raw.fixed[4] = 20)),
        ("page number", Box::new(|raw| raw.fixed[5] = 0x09)),
        ("sub-code", Box::new(|raw| (raw.fixed[7], raw.fixed[8]) = (0x3F, 0x7F))),
        ("context flags", Box::new(|raw| raw.fixed[9] = 0b1000_0000)),
        ("tick divisor", Box::new(|raw| raw.fixed[10] = MAX_TICK_DIVISOR + 1)),
        ("language", Box::new(|raw| raw.texts[0] = vec![b'a'; MAX_LANGUAGE_BYTES + 1])),
        ("channel address", Box::new(|raw| raw.texts[1] = vec![b'c'; MAX_URL_BYTES + 1])),
        ("page address", Box::new(|raw| raw.texts[2] = vec![b'p'; MAX_URL_BYTES + 1])),
        ("script address", Box::new(|raw| raw.texts[3] = vec![b's'; MAX_URL_BYTES + 1])),
        ("script address", Box::new(|raw| raw.texts[3] = vec![0xFF])),
        ("channel title", Box::new(|raw| raw.texts[4] = vec![b't'; MAX_TITLE_BYTES + 1])),
        ("channel title", Box::new(|raw| raw.texts[4] = vec![b'a', 0x07])),
        ("zone", Box::new(|raw| raw.zone = MAX_ZONE_MINUTES + 1)),
        ("too many keys", Box::new(|raw| raw.keys = (MAX_KEY_ASKS + 1, vec![b"k".to_vec(); MAX_KEY_ASKS + 1]))),
        ("key name", Box::new(|raw| raw.keys = (1, vec![vec![b'k'; MAX_KEY_NAME_BYTES + 1]]))),
        ("key name", Box::new(|raw| raw.keys = (1, vec![Vec::new()]))),
        ("too many sounds", Box::new(|raw| raw.sounds = (MAX_SOUND_HANDLES + 1, vec![1; MAX_SOUND_HANDLES + 1]))),
        ("source length", Box::new(|raw| raw.source = 0)),
        ("source length", Box::new(|raw| raw.source = u32::try_from(MAX_SCRIPT_BYTES + 1).unwrap())),
        ("payload ends early", Box::new(|raw| raw.sounds.0 += 1)),
        ("payload longer than its fields", Box::new(|raw| raw.sounds.1.push(0))),
    ];
    for (what, change) in wrong {
        let mut raw = RawOpen::new();
        change(&mut raw);
        assert!(refused(&raw.read(), what), "{what}: {:?}", raw.read());
    }
}

/// A piece of a source keeps to a piece's size and a script's length.
#[test]
fn a_source_piece_out_of_bounds_is_refused() {
    let piece =
        |offset: usize, length: usize| ToScriptHost::Source { context: ContextId::Page, offset: u32::try_from(offset).unwrap(), bytes: vec![b'x'; length] };
    let raw = |offset: usize, length: usize| [vec![2], u32::try_from(offset).unwrap().to_be_bytes().to_vec(), field(&vec![b'x'; length])].concat();
    for (what, offset, length) in [
        ("an empty piece", 0, 0),
        ("a piece one byte too long", 0, MAX_SOURCE_PIECE_BYTES + 1),
        ("a piece that ends one byte past a script's length", MAX_SCRIPT_BYTES - 9, 10),
        ("a piece that starts past any length", u32::MAX as usize, 1),
    ] {
        assert!(write_to_host(&mut Vec::new(), &piece(offset, length)).is_err(), "{what}");
        assert!(refused(&read_to_host(&mut frame(KIND_SOURCE, &raw(offset, length)).as_slice()), "source piece"), "{what}");
    }
    assert_eq!(read_to_host(&mut frame(KIND_SOURCE, &raw(MAX_SCRIPT_BYTES - 10, 10)).as_slice()).unwrap(), Some(piece(MAX_SCRIPT_BYTES - 10, 10)));
}

/// A step one past any bound is not written.
#[test]
fn a_step_one_past_a_bound_is_refused() {
    let wrong: Vec<(&str, Change<Step>)> = vec![
        ("too many events", Box::new(|step| step.events = vec![event(); MAX_STEP_EVENTS + 1])),
        ("event payload", Box::new(|step| step.events = vec![Event { payload: vec![0; MAX_EVENT_PAYLOAD_BYTES + 1], ..event() }])),
        ("too many painted", Box::new(|step| step.painted = vec![1; MAX_WATCHED_PAINTINGS + 1])),
        ("too many cycle ends", Box::new(|step| step.cycle_ended = vec![1; MAX_CYCLE_ENDS + 1])),
        ("too many bounds", Box::new(|step| step.bounds = vec![Bounds { handle: 1, x: 0, y: 0, width: 0, height: 0 }; MAX_TRACKED_DRAWINGS + 1])),
        ("too many sound reports", Box::new(|step| step.sound = vec![SoundReport::Ended { voice: 0 }; MAX_STEP_SOUND_REPORTS + 1])),
        ("too many faults", Box::new(|step| step.faults = vec![FaultReport { code: 1, bound: "B".into() }; MAX_STEP_FAULTS + 1])),
        ("bound name", Box::new(|step| step.faults = vec![FaultReport { code: 1, bound: "B".repeat(MAX_BOUND_NAME_BYTES + 1) }])),
        ("bound name", Box::new(|step| step.faults = vec![FaultReport { code: 1, bound: String::new() }])),
        ("bound name", Box::new(|step| step.faults = vec![FaultReport { code: 1, bound: "max batch".into() }])),
    ];
    for (what, change) in wrong {
        let mut changed = step();
        change(&mut changed);
        assert!(refused(&changed.check(), what), "{what}");
        assert!(refused(&write_to_host(&mut Vec::new(), &ToScriptHost::Step(Box::new(changed))), what), "{what}");
    }
}

/// A step's payload written by hand: its numbers, its flags, then its six
/// lists, each a count and its items' bytes.
fn raw_step(flags: u8, lists: &[(usize, Vec<u8>); 6]) -> Vec<u8> {
    let mut payload = [41_u32.to_be_bytes().as_slice(), &7_u64.to_be_bytes(), &0_u16.to_be_bytes(), &[flags]].concat();
    for (count, items) in lists {
        payload.extend_from_slice(&list(*count, items));
    }
    payload
}

/// The same bounds, read from bytes no writer of this crate made.
#[test]
fn a_step_one_past_a_bound_is_not_read() {
    let none = || -> [(usize, Vec<u8>); 6] { Default::default() };
    let raw_event = |kind: u8, payload: usize| [vec![kind], vec![0; 16], field(&vec![0; payload])].concat();
    let read = |flags: u8, lists: &[(usize, Vec<u8>); 6]| read_to_host(&mut frame(KIND_STEP, &raw_step(flags, lists)).as_slice());
    assert_eq!(
        read(0, &none()).unwrap(),
        Some(ToScriptHost::Step(Box::new(Step { sequence: 41, tick: 7, ..Step::default() }))),
        "the hand's layout is the reader's"
    );
    // Each list in turn, one past its bound, with as many whole items.
    let items: [(&str, usize, Vec<u8>); 6] = [
        ("too many events", MAX_STEP_EVENTS, raw_event(1, 0)),
        ("too many painted", MAX_WATCHED_PAINTINGS, vec![0; 4]),
        ("too many cycle ends", MAX_CYCLE_ENDS, vec![0; 4]),
        ("too many bounds", MAX_TRACKED_DRAWINGS, vec![0; 20]),
        ("too many sound reports", MAX_STEP_SOUND_REPORTS, vec![2, 0]),
        ("too many faults", MAX_STEP_FAULTS, [vec![0; 4], field(b"B")].concat()),
    ];
    for (place, (what, most, item)) in items.iter().enumerate() {
        let mut lists = none();
        lists[place] = (*most, item.repeat(*most));
        assert!(read(0, &lists).is_ok(), "{what}: the bound itself is read");
        lists[place] = (most + 1, item.repeat(most + 1));
        assert!(refused(&read(0, &lists), what), "{what}: {:?}", read(0, &lists));
    }
    let with_events = |events: Vec<u8>| {
        let mut lists = none();
        lists[0] = (1, events);
        lists
    };
    assert!(read(0, &with_events(raw_event(21, MAX_EVENT_PAYLOAD_BYTES))).is_ok(), "the longest payload");
    assert!(refused(&read(0, &with_events(raw_event(21, MAX_EVENT_PAYLOAD_BYTES + 1))), "event payload"));
    assert!(refused(&read(0, &with_events(raw_event(0, 0))), "event kind"));
    // The script host's own event 22 (sound is allowed) is never a
    // step's: the step's flag says it. The network's events are the web
    // client's messages.
    assert!(refused(&read(0, &with_events(raw_event(22, 0))), "event kind"));
    assert!(refused(&read(0, &with_events(raw_event(30, 0))), "event kind"));
    assert!(read(0b1_0000, &none()).is_ok(), "sound is allowed");
    assert!(refused(&read(0b10_0000, &none()), "step flags"));
    let mut lists = none();
    lists[4] = (1, vec![3, 0]);
    assert!(refused(&read(0, &lists), "sound report"));
    lists = none();
    lists[5] = (1, [vec![0; 4], field(&[b'B'; MAX_BOUND_NAME_BYTES + 1])].concat());
    assert!(refused(&read(0, &lists), "bound name"));
    lists = none();
    lists[5] = (0, vec![0]);
    assert!(refused(&read(0, &lists), "payload longer than its fields"));
}

/// A whole message cut anywhere, or with a byte after it, is refused.
#[test]
fn a_message_cut_short_or_running_on_is_refused() {
    for message in [ToScriptHost::ContextOpen(Box::new(open())), ToScriptHost::Step(Box::new(step())), ToScriptHost::Hello { link_version: 1 }] {
        let mut pipe = Vec::new();
        write_to_host(&mut pipe, &message).unwrap();
        let (kind, payload) = (pipe[0], &pipe[HEADER_BYTES..]);
        for cut in 0..payload.len() {
            assert!(read_to_host(&mut frame(kind, &payload[..cut]).as_slice()).is_err(), "{message:?} cut at {cut}");
        }
        assert!(read_to_host(&mut frame(kind, &[payload, &[0]].concat()).as_slice()).is_err(), "{message:?} running on");
    }
    for message in [
        FromScriptHost::Batch(Box::new(batch())),
        FromScriptHost::Ready { link_version: 1, api_max: 1 },
        FromScriptHost::Failed { context: ContextId::Page, reason: FailReason::Trap, text: "trap".into() },
    ] {
        let mut pipe = Vec::new();
        write_to_decoder(&mut pipe, &message).unwrap();
        let (kind, payload) = (pipe[0], &pipe[HEADER_BYTES..]);
        for cut in 0..payload.len() {
            // A batch cut between two sections is a batch of fewer
            // sections; cut anywhere else it is refused. Either way it is
            // never the message that was written.
            let read = read_to_decoder(&mut frame(kind, &payload[..cut]).as_slice());
            assert!(read.as_ref().map_or(true, |read| read.as_ref() != Some(&message)), "{message:?} cut at {cut}");
            if !matches!(message, FromScriptHost::Batch(_)) {
                assert!(read.is_err(), "{message:?} cut at {cut}");
            }
        }
        assert!(read_to_decoder(&mut frame(kind, &[payload, &[0]].concat()).as_slice()).is_err(), "{message:?} running on");
    }
    assert!(read_to_host(&mut frame(KIND_SHUTDOWN, &[0]).as_slice()).is_err(), "a shutdown with a payload");
    assert!(read_to_host(&mut frame(KIND_CONTEXT_CLOSE, &[3]).as_slice()).is_err(), "an unknown context");
    let failed = |reason: u8, text: &[u8]| read_to_decoder(&mut frame(KIND_FAILED, &[vec![2, reason], field(text)].concat()).as_slice());
    assert!(failed(6, b"link").is_ok());
    assert!(refused(&failed(0, b""), "failure reason"));
    assert!(refused(&failed(7, b""), "failure reason"));
    assert!(refused(&failed(1, "x".repeat(MAX_LOG_CHARS + 1).as_bytes()), "log line"));
    assert!(refused(&failed(1, &vec![b'x'; MAX_LOG_BYTES + 1]), "log line"));
    assert!(refused(&failed(1, b"a\nb"), "log line"));
    assert!(
        write_to_decoder(&mut Vec::new(), &FromScriptHost::Failed { context: ContextId::Page, reason: FailReason::Time, text: "x".repeat(MAX_LOG_CHARS + 1) })
            .is_err()
    );
}

/// A batch's sections come at most once and in their order; one that is
/// unknown, twice, out of place or longer than its fields refuses the
/// whole batch.
#[test]
fn a_batch_holds_each_section_once_and_in_order() {
    let sections = sections_of(&batch());
    assert_eq!(sections.iter().map(|(tag, _)| *tag).collect::<Vec<u8>>(), (1..=17).collect::<Vec<u8>>(), "every section, in its order");
    assert_eq!(read_batch(&raw_batch(&sections)).unwrap(), Some(FromScriptHost::Batch(Box::new(batch()))));
    for place in 0..sections.len() {
        let mut twice = sections.clone();
        twice.insert(place, sections[place].clone());
        assert!(refused(&read_batch(&raw_batch(&twice)), "section order"), "section {} twice", sections[place].0);
        if place > 0 {
            let mut swapped = sections.clone();
            swapped.swap(place - 1, place);
            assert!(refused(&read_batch(&raw_batch(&swapped)), "section order"), "section {} before its place", sections[place].0);
        }
        // Each alone is a batch.
        assert!(read_batch(&raw_batch(&sections[place..=place])).is_ok(), "section {} alone", sections[place].0);
    }
    for unknown in [0, 18, 23, 24, 36, 255] {
        let read = read_batch(&raw_batch(&[(unknown, Vec::new())]));
        assert!(refused(&read, if unknown == 0 { "section order" } else { "section" }), "tag {unknown}: {read:?}");
    }
    // Bytes after a section's fields, for every section whose fields end.
    for (tag, body) in &sections {
        if ![VERDICTS, REGIS, NAPLPS, TITLE, NAVIGATE].contains(tag) {
            let longer = [body.as_slice(), &[0]].concat();
            assert!(read_batch(&raw_batch(&[(*tag, longer)])).is_err(), "section {tag} running on");
        }
    }
    // A section whose length runs past the batch.
    let mut short = raw_batch(&[(TITLE, b"title".to_vec())]);
    short.pop();
    assert!(refused(&read_batch(&short), "payload ends early"));
}

/// A batch of more than the bound is not written, and no frame can carry
/// one to be read.
#[test]
fn a_batch_one_byte_too_long_is_refused() {
    // The sequence, a tag and a length come before the text.
    let fits = MAX_BATCH_BYTES - 4 - 1 - 4;
    let whole = FromScriptHost::Batch(Box::new(Batch { regis: Some(vec![b'P'; fits]), ..Batch::new(1) }));
    let mut pipe = Vec::new();
    write_to_decoder(&mut pipe, &whole).unwrap();
    assert_eq!(pipe.len(), HEADER_BYTES + MAX_BATCH_BYTES);
    assert_eq!(read_to_decoder(&mut pipe.as_slice()).unwrap(), Some(whole));
    let over = FromScriptHost::Batch(Box::new(Batch { regis: Some(vec![b'P'; fits + 1]), ..Batch::new(1) }));
    assert!(matches!(write_to_decoder(&mut Vec::new(), &over), Err(ProtocolError::TooLong(_))));
    let mut long = vec![KIND_BATCH];
    long.extend_from_slice(&u32::try_from(MAX_BATCH_BYTES + 1).unwrap().to_be_bytes());
    long.extend_from_slice(&vec![0; MAX_BATCH_BYTES + 1]);
    assert!(matches!(read_to_decoder(&mut long.as_slice()), Err(ProtocolError::TooLong(_))));
}

/// One section's list at its bound and one past it. `item` is the bytes
/// of one item; the list is a count and its items.
struct Counted {
    what: &'static str,
    tag: u8,
    most: usize,
    /// The bytes before the count.
    before: Vec<u8>,
    item: Vec<u8>,
}

/// Every counted list of a batch: one past its bound is refused by name,
/// before an item is read.
#[test]
fn a_section_with_one_item_too_many_is_not_read() {
    let label = [vec![0; 4], field(b"L")].concat();
    let log = [vec![0, 1], field(b"line")].concat();
    let counted = [
        Counted { what: "too many rows", tag: TELETEXT_ROWS, most: MAX_SCRIPT_ROWS, before: Vec::new(), item: [vec![1], vec![0x20; 40]].concat() },
        Counted { what: "too many triplets", tag: TELETEXT_ENHANCEMENT, most: MAX_LOCAL_TRIPLETS, before: Vec::new(), item: vec![0; 3] },
        Counted { what: "too many colours", tag: TELETEXT_SETTINGS, most: MAX_DEFINABLE_COLOURS, before: vec![0, 0, 0], item: vec![16, 0, 0] },
        Counted { what: "too many objects", tag: TELETEXT_OBJECTS, most: MAX_TELETEXT_OBJECTS, before: Vec::new(), item: vec![0; 4] },
        Counted { what: "too many sprites", tag: SPRITES, most: MAX_SPRITES, before: Vec::new(), item: vec![0, 1, 0, 1, 0, 0, 0, 1, 1, 0, 1, 0, 0] },
        Counted { what: "animate operations", tag: ANIMATE, most: MAX_ANIMATE_OPERATIONS, before: Vec::new(), item: vec![5, 2, 0, 1] },
        Counted { what: "too many objects", tag: NAPLPS_OBJECTS, most: MAX_NAPLPS_OBJECTS, before: [vec![0], field(b"")].concat(), item: Vec::new() },
        Counted { what: "too many sound operations", tag: SOUND, most: MAX_SOUND_OPERATIONS, before: Vec::new(), item: vec![2, 0] },
        Counted { what: "too many watched", tag: WATCH, most: MAX_WATCHED_PAINTINGS, before: vec![0b001], item: vec![0; 4] },
        Counted { what: "too many watched", tag: WATCH, most: MAX_TRACKED_DRAWINGS, before: vec![0b010], item: vec![0; 4] },
        Counted { what: "too many watched", tag: WATCH, most: MAX_WHEEL_THINGS, before: vec![0b100], item: vec![0; 4] },
        Counted { what: "too many labels", tag: LABELS, most: MAX_LABELS, before: Vec::new(), item: label },
        Counted { what: "too many log lines", tag: LOG, most: MAX_LOG_LINES, before: Vec::new(), item: log },
        Counted { what: "too many contexts", tag: STATE, most: CONTEXTS, before: Vec::new(), item: [vec![1, 0, 0, 0, 0], vec![0; 8]].concat() },
    ];
    for Counted { what, tag, most, before, item } in counted {
        let over = [before.as_slice(), &list(most + 1, &item.repeat(most + 1))].concat();
        let read = read_batch(&raw_batch(&[(tag, over)]));
        assert!(refused(&read, what), "section {tag}, {what}: {read:?}");
    }
    let verdicts = read_batch(&raw_batch(&[(VERDICTS, vec![0; MAX_STEP_EVENTS + 1])]));
    assert!(refused(&verdicts, "too many verdicts"), "{verdicts:?}");
    assert!(read_batch(&raw_batch(&[(VERDICTS, vec![3; MAX_STEP_EVENTS])])).is_ok());
    // An object's own triplets, and a sprite's and an object's frames.
    let triplets = [vec![0, 0], list(MAX_OBJECT_TRIPLETS + 1, &vec![0; 3 * (MAX_OBJECT_TRIPLETS + 1)])].concat();
    assert!(refused(&read_batch(&raw_batch(&[(TELETEXT_OBJECTS, list(1, &triplets))])), "too many triplets"));
    let frames = [vec![0, 1, 0, 1, 0, 0, 0, 1, 1], list(MAX_OBJECT_FRAMES + 1, &[0; 2 * (MAX_OBJECT_FRAMES + 1)])].concat();
    assert!(refused(&read_batch(&raw_batch(&[(SPRITES, list(1, &frames))])), "sprite frames"));
}

/// Every list at its bound is written and read back.
#[test]
fn a_section_at_its_bound_round_trips() {
    let most = |count: usize| u16::try_from(count).unwrap();
    let rows: Vec<TeletextRow> = (FIRST_SCRIPT_ROW..=LAST_SCRIPT_ROW).map(|row| TeletextRow { row, codes: [0x41; 40] }).collect();
    assert_eq!(rows.len(), MAX_SCRIPT_ROWS);
    let full = [
        Batch { verdicts: Some(vec![Verdict::default(); MAX_STEP_EVENTS]), ..Batch::new(1) },
        Batch { teletext_rows: Some(rows), ..Batch::new(1) },
        Batch { teletext_enhancement: Some(vec![Triplet { address: 1, mode: 1, data: 1 }; MAX_LOCAL_TRIPLETS]), ..Batch::new(1) },
        Batch {
            teletext_settings: Some(TeletextSettings {
                colours: (FIRST_DEFINABLE_ENTRY..=LAST_COLOUR_ENTRY).map(|entry| (entry, 0x123)).collect(),
                ..TeletextSettings::default()
            }),
            ..Batch::new(1)
        },
        Batch {
            teletext_objects: Some(
                (0..most(MAX_TELETEXT_OBJECTS)).map(|number| TeletextObject { number, triplets: vec![Triplet { address: 1, mode: 1, data: 1 }; 8] }).collect(),
            ),
            ..Batch::new(1)
        },
        Batch {
            teletext_objects: Some(vec![TeletextObject { number: 1, triplets: vec![Triplet { address: 1, mode: 1, data: 1 }; MAX_OBJECT_TRIPLETS] }]),
            ..Batch::new(1)
        },
        Batch {
            sprites: Some((1..=most(MAX_SPRITES)).map(|number| Sprite { objects: vec![1; MAX_OBJECT_FRAMES], ..sprite(number) }).collect()),
            ..Batch::new(1)
        },
        Batch { animate: Some(vec![operations()[6]; MAX_ANIMATE_OPERATIONS]), ..Batch::new(1) },
        Batch {
            naplps_objects: Some(NaplpsObjects {
                one: false,
                title: "t".repeat(MAX_TITLE_BYTES),
                objects: (1..=most(MAX_NAPLPS_OBJECTS))
                    .map(|number| NaplpsObject { frames: vec![FIRST_MACRO; MAX_OBJECT_FRAMES], label: "l".repeat(MAX_LABEL_BYTES), ..naplps_object(number) })
                    .collect(),
            }),
            ..Batch::new(1)
        },
        Batch { sound: Some(vec![sounds()[3]; MAX_SOUND_OPERATIONS]), ..Batch::new(1) },
        Batch {
            watch: Some(Watch {
                painted: Some(vec![1; MAX_WATCHED_PAINTINGS]),
                tracked: Some(vec![2; MAX_TRACKED_DRAWINGS]),
                wheel: Some(vec![3; MAX_WHEEL_THINGS]),
            }),
            ..Batch::new(1)
        },
        Batch { labels: Some(vec![Label { handle: 1, label: "l".repeat(MAX_LABEL_BYTES) }; MAX_LABELS]), ..Batch::new(1) },
        Batch { title: Some("t".repeat(MAX_TITLE_BYTES)), ..Batch::new(1) },
        Batch { navigate: Some("n".repeat(MAX_URL_BYTES)), ..Batch::new(1) },
        Batch { log: Some(vec![LogLine { level: 0, context: ContextId::Page, text: "\u{1F600}".repeat(MAX_LOG_CHARS) }; MAX_LOG_LINES]), ..Batch::new(1) },
    ];
    for batch in full {
        let message = FromScriptHost::Batch(Box::new(batch));
        let mut pipe = Vec::new();
        write_to_decoder(&mut pipe, &message).unwrap_or_else(|error| panic!("{error}: {message:?}"));
        assert_eq!(read_to_decoder(&mut pipe.as_slice()).unwrap(), Some(message));
    }
}

/// Holds that each change to a whole batch makes one no writer writes,
/// refused by the name given.
fn refuse_each(wrong: Vec<(&str, Change<Batch>)>) {
    for (what, change) in wrong {
        let mut changed = batch();
        change(&mut changed);
        let written = write_to_decoder(&mut Vec::new(), &FromScriptHost::Batch(Box::new(changed)));
        assert!(refused(&written, what), "{what}: {written:?}");
    }
}

/// A batch with one value of a teletext section one past its bound, or
/// of a shape no section has, is not written.
#[test]
fn a_teletext_section_one_past_a_bound_is_not_written() {
    let triplet = Triplet { address: 0, mode: 0, data: 0 };
    let wrong: Vec<(&str, Change<Batch>)> = vec![
        ("too many verdicts", Box::new(|batch| batch.verdicts = Some(vec![Verdict::default(); MAX_STEP_EVENTS + 1]))),
        ("too many rows", Box::new(|batch| batch.teletext_rows = Some(vec![TeletextRow { row: 1, codes: [0; 40] }; MAX_SCRIPT_ROWS + 1]))),
        ("row", Box::new(|batch| batch.teletext_rows = Some(vec![TeletextRow { row: 0, codes: [0; 40] }]))),
        ("row", Box::new(|batch| batch.teletext_rows = Some(vec![TeletextRow { row: LAST_SCRIPT_ROW + 1, codes: [0; 40] }]))),
        (
            "rows out of order",
            Box::new(|batch| batch.teletext_rows = Some(vec![TeletextRow { row: 2, codes: [0; 40] }, TeletextRow { row: 2, codes: [0; 40] }])),
        ),
        (
            "rows out of order",
            Box::new(|batch| batch.teletext_rows = Some(vec![TeletextRow { row: 3, codes: [0; 40] }, TeletextRow { row: 2, codes: [0; 40] }])),
        ),
        ("row code", Box::new(|batch| batch.teletext_rows = Some(vec![TeletextRow { row: 1, codes: [MAX_ROW_CODE + 1; 40] }]))),
        ("too many triplets", Box::new(move |batch| batch.teletext_enhancement = Some(vec![triplet; MAX_LOCAL_TRIPLETS + 1]))),
        ("triplet", Box::new(move |batch| batch.teletext_enhancement = Some(vec![Triplet { address: MAX_TRIPLET_ADDRESS + 1, ..triplet }]))),
        ("triplet", Box::new(move |batch| batch.teletext_enhancement = Some(vec![Triplet { mode: MAX_TRIPLET_MODE + 1, ..triplet }]))),
        ("triplet", Box::new(move |batch| batch.teletext_enhancement = Some(vec![Triplet { data: MAX_TRIPLET_DATA + 1, ..triplet }]))),
        ("default colour", Box::new(|batch| batch.teletext_settings.as_mut().unwrap().screen_colour = LAST_COLOUR_ENTRY + 1)),
        ("default colour", Box::new(|batch| batch.teletext_settings.as_mut().unwrap().row_colour = LAST_COLOUR_ENTRY + 1)),
        ("too many colours", Box::new(|batch| batch.teletext_settings.as_mut().unwrap().colours = vec![(16, 0); MAX_DEFINABLE_COLOURS + 1])),
        ("colour", Box::new(|batch| batch.teletext_settings.as_mut().unwrap().colours = vec![(FIRST_DEFINABLE_ENTRY - 1, 0)])),
        ("colour", Box::new(|batch| batch.teletext_settings.as_mut().unwrap().colours = vec![(LAST_COLOUR_ENTRY + 1, 0)])),
        ("colour", Box::new(|batch| batch.teletext_settings.as_mut().unwrap().colours = vec![(16, MAX_RGB12 + 1)])),
        ("colours out of order", Box::new(|batch| batch.teletext_settings.as_mut().unwrap().colours = vec![(17, 0), (17, 1)])),
        (
            "too many objects",
            Box::new(|batch| batch.teletext_objects = Some(vec![TeletextObject { number: 0, triplets: Vec::new() }; MAX_TELETEXT_OBJECTS + 1])),
        ),
        ("object number", Box::new(|batch| batch.teletext_objects = Some(vec![TeletextObject { number: MAX_OBJECT_NUMBER + 1, triplets: Vec::new() }]))),
        ("objects out of order", Box::new(|batch| batch.teletext_objects.as_mut().unwrap().reverse())),
        (
            "too many triplets",
            Box::new(move |batch| batch.teletext_objects = Some(vec![TeletextObject { number: 0, triplets: vec![triplet; MAX_OBJECT_TRIPLETS + 1] }])),
        ),
        ("too many sprites", Box::new(|batch| batch.sprites = Some((1..=u16::try_from(MAX_SPRITES + 1).unwrap()).map(sprite).collect()))),
        ("sprite number", Box::new(|batch| batch.sprites = Some(vec![sprite(0)]))),
        ("sprite numbers", Box::new(|batch| batch.sprites = Some(vec![sprite(4), sprite(5), sprite(4)]))),
        ("sprite layer", Box::new(|batch| batch.sprites = Some(vec![Sprite { layer: 0, ..sprite(1) }]))),
        ("sprite layer", Box::new(|batch| batch.sprites = Some(vec![Sprite { layer: 4, ..sprite(1) }]))),
        ("sprite origin", Box::new(|batch| batch.sprites = Some(vec![Sprite { column: 40, ..sprite(1) }]))),
        ("sprite origin", Box::new(|batch| batch.sprites = Some(vec![Sprite { column: -40, ..sprite(1) }]))),
        ("sprite origin", Box::new(|batch| batch.sprites = Some(vec![Sprite { row: 25, ..sprite(1) }]))),
        ("sprite origin", Box::new(|batch| batch.sprites = Some(vec![Sprite { row: i8::MIN, ..sprite(1) }]))),
        ("sprite size", Box::new(|batch| batch.sprites = Some(vec![Sprite { width: 0, ..sprite(1) }]))),
        ("sprite size", Box::new(|batch| batch.sprites = Some(vec![Sprite { width: MAX_SPRITE_WIDTH + 1, ..sprite(1) }]))),
        ("sprite size", Box::new(|batch| batch.sprites = Some(vec![Sprite { height: MAX_SPRITE_HEIGHT + 1, ..sprite(1) }]))),
        ("sprite frames", Box::new(|batch| batch.sprites = Some(vec![Sprite { objects: Vec::new(), ..sprite(1) }]))),
        ("sprite frames", Box::new(|batch| batch.sprites = Some(vec![Sprite { objects: vec![1; MAX_OBJECT_FRAMES + 1], ..sprite(1) }]))),
        ("object number", Box::new(|batch| batch.sprites = Some(vec![Sprite { objects: vec![MAX_OBJECT_NUMBER + 1], ..sprite(1) }]))),
    ];
    refuse_each(wrong);
}

/// The same for the animation operations and the NAPLPS objects.
#[test]
fn a_drawing_section_one_past_a_bound_is_not_written() {
    let wrong: Vec<(&str, Change<Batch>)> = vec![
        ("animate operations", Box::new(|batch| batch.animate = Some(Vec::new()))),
        ("animate operations", Box::new(|batch| batch.animate = Some(vec![operations()[0]; MAX_ANIMATE_OPERATIONS + 1]))),
        ("animate operation", Box::new(|batch| batch.animate = Some(vec![Operation { target: object(2), kind: OperationKind::Reset }]))),
        ("animate operation", Box::new(|batch| batch.animate = Some(vec![Operation { target: Target::PAGE, kind: OperationKind::Show }]))),
        ("animate operation", Box::new(|batch| batch.animate = Some(vec![Operation { target: object(2), kind: OperationKind::Frame(16) }]))),
        (
            "too many objects",
            Box::new(|batch| batch.naplps_objects.as_mut().unwrap().objects = (1..=u16::try_from(MAX_NAPLPS_OBJECTS + 1).unwrap()).map(region).collect()),
        ),
        ("one object", Box::new(|batch| batch.naplps_objects.as_mut().unwrap().one = true)),
        ("one object", Box::new(|batch| batch.naplps_objects = Some(NaplpsObjects { one: true, title: "t".into(), objects: vec![region(1)] }))),
        ("title", Box::new(|batch| batch.naplps_objects.as_mut().unwrap().title = "t".repeat(MAX_TITLE_BYTES + 1))),
        ("title", Box::new(|batch| batch.naplps_objects.as_mut().unwrap().title = "a\u{9b}".into())),
        ("object numbers", Box::new(|batch| batch.naplps_objects.as_mut().unwrap().objects = vec![region(3), region(3)])),
        ("object number", Box::new(|batch| batch.naplps_objects.as_mut().unwrap().objects = vec![region(0)])),
        ("object layer", Box::new(|batch| batch.naplps_objects.as_mut().unwrap().objects[0].layer = 0)),
        ("object layer", Box::new(|batch| batch.naplps_objects.as_mut().unwrap().objects[0].layer = 4)),
        ("object size", Box::new(|batch| batch.naplps_objects.as_mut().unwrap().objects[0].width = RASTER_WIDTH + 1)),
        ("object size", Box::new(|batch| batch.naplps_objects.as_mut().unwrap().objects[0].height = RASTER_HEIGHT + 1)),
        ("object size", Box::new(|batch| batch.naplps_objects.as_mut().unwrap().objects[0].height = 0)),
        ("object look", Box::new(|batch| batch.naplps_objects.as_mut().unwrap().objects[0].looks[1] = FIRST_MACRO - 1)),
        ("object look", Box::new(|batch| batch.naplps_objects.as_mut().unwrap().objects[0].looks[2] = LAST_MACRO + 1)),
        ("object frames", Box::new(|batch| batch.naplps_objects.as_mut().unwrap().objects[0].frames = Vec::new())),
        ("object frames", Box::new(|batch| batch.naplps_objects.as_mut().unwrap().objects[0].frames = vec![FIRST_MACRO; MAX_OBJECT_FRAMES + 1])),
        ("object frames", Box::new(|batch| batch.naplps_objects.as_mut().unwrap().objects[0].frames = vec![FIRST_MACRO - 1])),
        ("object frames", Box::new(|batch| batch.naplps_objects.as_mut().unwrap().objects[1].frames = vec![FIRST_MACRO])),
        ("label", Box::new(|batch| batch.naplps_objects.as_mut().unwrap().objects[0].label = "l".repeat(MAX_LABEL_BYTES + 1))),
        ("label", Box::new(|batch| batch.naplps_objects.as_mut().unwrap().objects[0].label = "a\tb".into())),
    ];
    refuse_each(wrong);
}

/// The same for the sound, the watch lists, the labels, the title, the
/// navigation, the log and the contexts' state.
#[test]
fn another_section_one_past_a_bound_is_not_written() {
    let wrong: Vec<(&str, Change<Batch>)> = vec![
        ("too many sound operations", Box::new(|batch| batch.sound = Some(vec![SoundOperation::Stop { voice: 0 }; MAX_SOUND_OPERATIONS + 1]))),
        ("sound operation", Box::new(|batch| batch.sound = Some(vec![SoundOperation::Stop { voice: u8::try_from(AUDIO_CHANNELS).unwrap() }]))),
        ("sound operation", Box::new(|batch| batch.sound = Some(vec![SoundOperation::Play { voice: 0, sound: 1, gain: MAX_GAIN + 1 }]))),
        ("sound operation", Box::new(|batch| batch.sound = Some(vec![SoundOperation::SynthStop { part: MAX_PART + 1 }]))),
        ("sound operation", Box::new(|batch| batch.sound = Some(vec![SoundOperation::SynthStart { part: 0, family: FIRST_FAMILY - 1, variant: 0, gain: 0 }]))),
        ("sound operation", Box::new(|batch| batch.sound = Some(vec![SoundOperation::SynthStart { part: 0, family: LAST_FAMILY + 1, variant: 0, gain: 0 }]))),
        (
            "sound operation",
            Box::new(|batch| batch.sound = Some(vec![SoundOperation::SynthStart { part: 0, family: FIRST_FAMILY, variant: MAX_VARIANT + 1, gain: 0 }])),
        ),
        ("sound operation", Box::new(|batch| batch.sound = Some(vec![SoundOperation::Midi { part: 0, status: 0xF0, data1: 0, data2: 0 }]))),
        ("sound operation", Box::new(|batch| batch.sound = Some(vec![SoundOperation::Midi { part: 0, status: 0x91, data1: 0, data2: 0 }]))),
        ("sound operation", Box::new(|batch| batch.sound = Some(vec![SoundOperation::Midi { part: 0, status: 0x90, data1: MAX_MIDI_DATA + 1, data2: 0 }]))),
        ("sound operation", Box::new(|batch| batch.sound = Some(vec![SoundOperation::Midi { part: 0, status: 0x90, data1: 0, data2: MAX_MIDI_DATA + 1 }]))),
        ("watch lists", Box::new(|batch| batch.watch = Some(Watch::default()))),
        ("too many watched", Box::new(|batch| batch.watch = Some(Watch { painted: Some(vec![1; MAX_WATCHED_PAINTINGS + 1]), ..Watch::default() }))),
        ("too many watched", Box::new(|batch| batch.watch = Some(Watch { tracked: Some(vec![1; MAX_TRACKED_DRAWINGS + 1]), ..Watch::default() }))),
        ("too many watched", Box::new(|batch| batch.watch = Some(Watch { wheel: Some(vec![1; MAX_WHEEL_THINGS + 1]), ..Watch::default() }))),
        ("too many labels", Box::new(|batch| batch.labels = Some(vec![Label { handle: 1, label: "l".into() }; MAX_LABELS + 1]))),
        ("label", Box::new(|batch| batch.labels = Some(vec![Label { handle: 1, label: "l".repeat(MAX_LABEL_BYTES + 1) }]))),
        ("label", Box::new(|batch| batch.labels = Some(vec![Label { handle: 1, label: "a\u{0}".into() }]))),
        ("title", Box::new(|batch| batch.title = Some("t".repeat(MAX_TITLE_BYTES + 1)))),
        ("title", Box::new(|batch| batch.title = Some("a\r\nb".into()))),
        ("navigation target", Box::new(|batch| batch.navigate = Some(String::new()))),
        ("navigation target", Box::new(|batch| batch.navigate = Some("n".repeat(MAX_URL_BYTES + 1)))),
        ("navigation target", Box::new(|batch| batch.navigate = Some("100\n".into()))),
        (
            "too many log lines",
            Box::new(|batch| batch.log = Some(vec![LogLine { level: 0, context: ContextId::Page, text: String::new() }; MAX_LOG_LINES + 1])),
        ),
        ("log level", Box::new(|batch| batch.log.as_mut().unwrap()[0].level = MAX_LOG_LEVEL + 1)),
        ("log line", Box::new(|batch| batch.log.as_mut().unwrap()[0].text = "x".repeat(MAX_LOG_CHARS + 1))),
        ("log line", Box::new(|batch| batch.log.as_mut().unwrap()[0].text = "a\u{1b}[31m".into())),
        (
            "too many contexts",
            Box::new(|batch| batch.state.as_mut().unwrap().push(ContextState { context: ContextId::Page, stopped: None, failures: 0, next_due: None })),
        ),
        ("contexts out of order", Box::new(|batch| batch.state.as_mut().unwrap().reverse())),
    ];
    refuse_each(wrong);
}

/// Bytes a subverted script host could write, section by section: each is
/// refused by the reader, which trusts no writer.
#[test]
fn a_section_of_a_shape_no_writer_makes_is_not_read() {
    let one = |batch: Batch| body_of(&batch);
    // A valid body with one byte changed, by its place.
    let patched = |batch: Batch, place: usize, byte: u8| {
        let (tag, mut body) = one(batch);
        body[place] = byte;
        read_batch(&raw_batch(&[(tag, body)]))
    };
    let rows = || Batch { teletext_rows: Some(vec![TeletextRow { row: 5, codes: [0x20; 40] }]), ..Batch::new(1) };
    assert!(refused(&patched(rows(), 2, 0), "row"), "row 0, the header, is never a script's");
    assert!(refused(&patched(rows(), 2, 24), "row"), "row 24 is never a script's");
    assert!(refused(&patched(rows(), 3, 0x80), "row code"));
    let triplets = || Batch { teletext_enhancement: Some(vec![Triplet { address: 1, mode: 1, data: 1 }]), ..Batch::new(1) };
    assert!(refused(&patched(triplets(), 2, 64), "triplet"));
    assert!(refused(&patched(triplets(), 3, 32), "triplet"));
    assert!(refused(&patched(triplets(), 4, 128), "triplet"));
    let settings = || Batch { teletext_settings: Some(TeletextSettings { colours: vec![(20, 0x0ABC)], ..TeletextSettings::default() }), ..Batch::new(1) };
    assert!(refused(&patched(settings(), 0, 32), "default colour"));
    assert!(refused(&patched(settings(), 2, 2), "substitution"));
    assert!(refused(&patched(settings(), 5, 15), "colour"));
    assert!(refused(&patched(settings(), 6, 0x10), "colour"));
    let sprites = || Batch { sprites: Some(vec![sprite(1)]), ..Batch::new(1) };
    assert!(refused(&patched(sprites(), 3, 0), "sprite number"));
    assert!(refused(&patched(sprites(), 4, 0b100), "sprite flags"));
    assert!(refused(&patched(sprites(), 5, 0), "sprite layer"));
    assert!(refused(&patched(sprites(), 7, 40), "sprite origin"));
    assert!(refused(&patched(sprites(), 9, 41), "sprite size"));
    assert!(refused(&patched(sprites(), 12, 0), "sprite frames"));
    assert!(refused(&patched(sprites(), 13, 0x02), "object number"));
    let animate = |kind| Batch { animate: Some(vec![Operation { target: object(2), kind }]), ..Batch::new(1) };
    assert!(refused(&patched(animate(OperationKind::Show), 2, 20), "animate operation"), "`reset` is never a script's");
    assert!(refused(&patched(animate(OperationKind::Show), 2, 4), "animate operation"), "a code no operation has");
    assert!(refused(&patched(animate(OperationKind::Show), 3, 0), "animate operation"), "the page is no target");
    assert!(refused(&patched(animate(OperationKind::Look(1)), 6, 4), "animate operation"));
    assert!(read_batch(&raw_batch(&[(ANIMATE, list(0, &[]))])).is_err(), "no operation");
    let (tag, mut body) = one(animate(OperationKind::Show));
    body.push(5);
    assert!(refused(&read_batch(&raw_batch(&[(tag, body)])), "payload longer than its fields"));
    let naplps = || Batch { naplps: Some(NaplpsAppend { new_generation: false, bytes: vec![1] }), ..Batch::new(1) };
    assert!(refused(&patched(naplps(), 0, 2), "new generation"));
    assert!(refused(&read_batch(&raw_batch(&[(NAPLPS, Vec::new())])), "payload ends early"));
    let objects = || Batch { naplps_objects: Some(NaplpsObjects { one: false, title: String::new(), objects: vec![naplps_object(1)] }), ..Batch::new(1) };
    // After the flag, the empty title's length and the count: the
    // object's number, kind, flags and layer.
    assert!(refused(&patched(objects(), 0, 2), "one object"));
    assert!(refused(&patched(objects(), 8, 0), "object number"));
    assert!(refused(&patched(objects(), 9, 3), "object kind"));
    assert!(refused(&patched(objects(), 9, 2), "object frames"), "a region with frames");
    assert!(refused(&patched(objects(), 10, 0b100), "object flags"));
    assert!(refused(&patched(objects(), 11, 0), "object layer"));
    let sound = || Batch { sound: Some(vec![SoundOperation::Midi { part: 0, status: 0x90, data1: 1, data2: 1 }]), ..Batch::new(1) };
    assert!(refused(&patched(sound(), 2, 6), "sound operation"));
    assert!(refused(&patched(sound(), 2, 0), "sound operation"));
    assert!(refused(&patched(sound(), 4, 0xA0), "sound operation"));
    let watch = || Batch { watch: Some(Watch { painted: Some(vec![1]), ..Watch::default() }), ..Batch::new(1) };
    assert!(refused(&patched(watch(), 0, 0), "watch lists"));
    assert!(refused(&patched(watch(), 0, 0b1000), "watch lists"));
    let log = || Batch { log: Some(vec![LogLine { level: 1, context: ContextId::Page, text: "a".into() }]), ..Batch::new(1) };
    assert!(refused(&patched(log(), 2, 5), "log level"));
    assert!(refused(&patched(log(), 3, 0), "context"));
    assert!(refused(&patched(log(), 8, 0x0A), "log line"));
    let state = || Batch { state: Some(vec![ContextState { context: ContextId::Page, stopped: None, failures: 0, next_due: None }]), ..Batch::new(1) };
    assert!(refused(&patched(state(), 2, 3), "context"));
    assert!(refused(&patched(state(), 3, 7), "failure reason"));
    assert!(refused(&patched(state(), 6, 2), "next due"));
    assert!(refused(&patched(state(), 14, 1), "next due"), "a tick with no timer");
    assert!(refused(&read_batch(&raw_batch(&[(VERDICTS, vec![0b100])])), "verdict"));
    assert!(refused(&read_batch(&raw_batch(&[(TITLE, vec![0xFF])])), "title"));
    assert!(refused(&read_batch(&raw_batch(&[(NAVIGATE, Vec::new())])), "navigation target"));
    assert!(refused(&read_batch(&raw_batch(&[(NAVIGATE, vec![b'n'; MAX_URL_BYTES + 1])])), "navigation target"));
    assert!(refused(&read_batch(&raw_batch(&[(TITLE, vec![b't'; MAX_TITLE_BYTES + 1])])), "title"));
}

/// The bounds this crate cannot take by name are the figures of the
/// crates that own them, and the flags pack as the specification orders
/// them.
#[test]
fn the_bounds_are_their_owners() {
    use neoteletext_web::records::naplps_objects as record;
    assert_eq!(MAX_BATCH_BYTES, neoteletext_web::consts::MAX_FRAME_BYTES);
    assert_eq!(MAX_NAPLPS_OBJECTS, record::MAX_OBJECTS);
    assert_eq!(MAX_LABEL_BYTES, record::MAX_LABEL_BYTES);
    assert_eq!(MAX_TITLE_BYTES, record::MAX_TITLE_BYTES);
    assert_eq!((FIRST_MACRO, LAST_MACRO), (record::FIRST_MACRO, record::LAST_MACRO));
    assert_eq!((RASTER_WIDTH, RASTER_HEIGHT), (record::RASTER_WIDTH, record::RASTER_HEIGHT));
    assert_eq!(MAX_OBJECT_FRAMES, record::MAX_OBJECT_FRAMES);
    assert_eq!(MAX_SPRITES, neoteletext_web::records::sprites::MAX_SPRITES);
    assert_eq!(MAX_WATCHED_PAINTINGS, regis::limits::MAX_WATCHED_PAINTINGS);
    assert_eq!(MAX_STEP_EVENTS, 32);
    assert_eq!((FIRST_SCRIPT_ROW, LAST_SCRIPT_ROW, MAX_SCRIPT_ROWS), (1, 23, 23));
    assert_eq!((MAX_LOCAL_TRIPLETS, MAX_OBJECT_TRIPLETS, MAX_TELETEXT_OBJECTS, MAX_OBJECT_NUMBER), (208, 507, 512, 511));
    assert_eq!(pack(ContextFlags { network: true, ..ContextFlags::default() }.list()), 0b01_0000);
    assert_eq!(pack(ContextFlags { from_disk: true, pointer_press: true, ..ContextFlags::default() }.list()), 0b10_0001);
    assert_eq!(pack(StepFlags { closing: true, tick: true, ..StepFlags::default() }.list()), 0b1001);
    // Sound allowed is the bit after the specification's own, in each.
    assert_eq!(pack(ContextFlags { sound_allowed: true, ..ContextFlags::default() }.list()), 0b100_0000);
    assert_eq!(pack(StepFlags { sound_allowed: true, ..StepFlags::default() }.list()), 0b1_0000);
    assert_eq!(ContextFlags::of(0b100_0000).unwrap(), ContextFlags { sound_allowed: true, ..ContextFlags::default() });
    assert_eq!(StepFlags::of(0b1_0000).unwrap(), StepFlags { sound_allowed: true, ..StepFlags::default() });
    assert_eq!(pack([true, false]), 1, "a verdict's bit 0 is the default cancelled");
}
