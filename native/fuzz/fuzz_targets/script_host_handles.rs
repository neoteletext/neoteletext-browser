//! The script library's host with the handles it gives fed back
//! (script-api.md sections 5 to 8 and 10.2, section 11.4 target 1; the
//! security review's finding S-06, TODO-7272): `script_host_ops` gives
//! every operation arbitrary numbers, which reach a live component,
//! sprite, label, drawing or NAPLPS object only by chance. Here each
//! number a call answers is kept and given back to later calls as a
//! script would give it, and the payloads are of the forms the
//! operations read: a component's options as JSON (operations 60 to 67),
//! a sprite's cells (112), painter programs (ReGIS 207 and 211, NAPLPS
//! 303 and 304), text with a language tag, and bitmap sixels (130 to
//! 138). The page is a teletext, ReGIS or NAPLPS page, with the channel
//! context beside it, and the calls are interleaved with steps, input,
//! scopes, reports of painting and contexts closing and opening again.
//! Nothing panics; the host's bytes stay within `MAX_HOST_STATE_BYTES`
//! and a step's output within `MAX_BATCH_BYTES`; and every batch is a
//! message the link takes.
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext_script::host::{Host, Payload, Reply};
use neoteletext_script::limits::{MAX_BATCH_BYTES, MAX_HOST_INTS, MAX_HOST_STATE_BYTES};
use neoteletext_script::ops::{EventKind, Op};
use protocol::script::{Bounds, ContextFlags, ContextId, ContextOpen, FromScriptHost, Level, Renderer, Step, write_to_decoder};

/// The most commands one input gives.
const MAX_COMMANDS: usize = 384;
/// The most handles kept.
const MAX_KEPT: usize = 128;
/// The longest payload of repeated input.
const MAX_PAYLOAD: usize = 12_000;
/// Whole numbers at the edges of the operations' ranges.
const EDGES: [i32; 16] = [0, 1, -1, 2, 7, 8, 23, 24, 39, 40, 255, 256, 65_535, 0x0002_0000, i32::MAX, i32::MIN];
/// Options of each kind of component, and some that are none.
const OPTIONS: [&str; 14] = [
    r#"{"title":"Page","hint":"Press","prompts":[{"text":"Red","target":"100"}]}"#,
    r#"{"text":"Hello","ink":7,"paper":4,"align":"center","id":"t"}"#,
    r#"{"lines":["a","b","c","d"],"more":"next","height":2}"#,
    r#"{"text":"Band","paper":1}"#,
    r#"{"text":"Go","target":"200"}"#,
    r#"{"text":"OK","id":"ok"}"#,
    r#"{"items":[{"text":"One"},{"text":"Two","disabled":true},{"text":"Three"}],"selected":1}"#,
    r#"{"columns":[{"title":"N","width":10},{"title":"S","width":5,"align":"right"}],"rows":[["a","1"],["b","2"]]}"#,
    r#"{"placeholder":"Type","value":"x","longest":20}"#,
    r#"{"on":true,"label":"Sound"}"#,
    r#"{"min":0,"max":10,"step":1,"value":5}"#,
    r#"{"value":30,"max":100}"#,
    r#"{"format":"utc"}"#,
    r#"{"nosuch":1}"#,
];
/// Language tags, with the NUL the text operations put after one.
const TAGS: [&[u8]; 4] = [b"en\0", b"de\0", b"\0", b"zz-qq\0"];

/// Reads the input a piece at a time; past its end every read is zero.
struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
    fn byte(&mut self) -> u8 {
        let Some((first, rest)) = self.0.split_first() else { return 0 };
        self.0 = rest;
        *first
    }

    fn int(&mut self) -> i32 {
        match self.byte() {
            choice @ 0..=99 => i32::from(choice) - 2,
            choice @ 100..=199 => EDGES[usize::from(choice) % EDGES.len()],
            _ => i32::from_le_bytes([self.byte(), self.byte(), self.byte(), self.byte()]),
        }
    }

    fn repeated(&mut self, length: usize) -> Vec<u8> {
        let piece = usize::from(self.byte() % 16) + 1;
        let (seed, rest) = self.0.split_at(piece.min(self.0.len()));
        self.0 = rest;
        if seed.is_empty() { vec![0; length] } else { seed.iter().copied().cycle().take(length).collect() }
    }

    fn bytes(&mut self) -> Vec<u8> {
        let length = (usize::from(self.byte()) * usize::from(self.byte().max(1))).min(MAX_PAYLOAD);
        self.repeated(length)
    }
}

/// A handle kept, or a number.
fn number(input: &mut Bytes<'_>, kept: &[i32]) -> i32 {
    if !kept.is_empty() && input.byte() < 112 { kept[usize::from(input.byte()) % kept.len()] } else { input.int() }
}

fn opening(context: ContextId, renderer: Renderer, flags: u8) -> ContextOpen {
    ContextOpen {
        context,
        api: 1,
        renderer,
        level: [Level::One, Level::OneFive, Level::TwoFive, Level::ThreeFive][usize::from(flags >> 6)],
        page: 0x100,
        subcode: 0,
        flags: ContextFlags {
            pointer_press: flags & 1 != 0,
            pointer_move: flags & 2 != 0,
            wheel: flags & 4 != 0,
            text: flags & 8 != 0,
            network: false,
            from_disk: false,
            sound_allowed: flags & 16 != 0,
        },
        tick_divisor: 1,
        language: "en".into(),
        channel_url: "https://example.test/chan/".into(),
        page_url: "https://example.test/chan/100.tti".into(),
        script_url: "https://example.test/chan/page.js".into(),
        channel_title: "Example".into(),
        seed: [flags; 32],
        epoch_seconds: 1_800_000_000,
        zone_minutes: 0,
        keys: vec!["Enter".into(), "a".into()],
        sounds: vec![1, 2],
        source_bytes: 10,
    }
}

/// The operations of a page of `renderer`, the components' and the core's.
fn operations(renderer: Renderer) -> Vec<Op> {
    let own = match renderer {
        Renderer::Teletext => 100..140,
        Renderer::Regis => 200..216,
        Renderer::Naplps => 300..313,
    };
    Op::ALL.iter().map(|(_, op)| *op).filter(|op| own.contains(&op.number()) || (60..68).contains(&op.number()) || (9..22).contains(&op.number())).collect()
}

/// A payload of the form `op` reads, or now and then any bytes.
fn payload_for(op: Op, input: &mut Bytes<'_>, kept: &[i32]) -> (u8, Vec<u8>) {
    if input.byte() < 32 {
        return (input.byte() % 4, input.bytes());
    }
    match op.number() {
        // A component's options, or a key or text it is offered.
        60 | 61 => (1, OPTIONS[usize::from(input.byte()) % OPTIONS.len()].as_bytes().to_vec()),
        66 => (1, if input.byte() & 1 == 0 { b"Enter".to_vec() } else { b"abc".to_vec() }),
        // Words of a painter program, or frames, or many handles' values:
        // whole numbers, little-endian, among them handles kept.
        112 | 136 => {
            let length = usize::from(input.byte()) * 6;
            (2, input.repeated(length))
        }
        114 | 205 | 207 | 211 | 215 | 303 | 304 | 308 | 310 => {
            let words = usize::from(input.byte()) + 1;
            let ints: Vec<u8> = (0..words).flat_map(|_| number(input, kept).to_le_bytes()).collect();
            (3, ints)
        }
        // Text with a language tag before it.
        200 | 206 | 212 | 300 | 309 | 312 | 113 | 120 | 137 => {
            let mut text = TAGS[usize::from(input.byte()) % TAGS.len()].to_vec();
            text.extend(String::from_utf8_lossy(&input.bytes()).chars().take(200).collect::<String>().bytes());
            (1, text)
        }
        _ => (input.byte() % 4, input.bytes()),
    }
}

fuzz_target!(|data: &[u8]| {
    let mut input = Bytes(data);
    let renderer = [Renderer::Teletext, Renderer::Regis, Renderer::Naplps][usize::from(input.byte() % 3)];
    let operations = operations(renderer);
    let mut host = Host::new();
    host.open(opening(ContextId::Channel, renderer, input.byte()), 10);
    host.open(opening(ContextId::Page, renderer, input.byte()), 10);
    let mut kept: Vec<i32> = Vec::new();
    let mut tick = 0_u64;
    let mut sequence = 0_u32;
    host.begin_step(tick, true);
    host.begin_callback(ContextId::Page, None, 0);
    for _ in 0..MAX_COMMANDS {
        if input.0.is_empty() {
            break;
        }
        match input.byte() % 20 {
            0 => {
                host.end_callback();
                let batch = host.take_batch(sequence, &[]);
                sequence = sequence.wrapping_add(1);
                if let Err(error) = write_to_decoder(&mut Vec::new(), &FromScriptHost::Batch(Box::new(batch.clone()))) {
                    panic!("the batch is not a message the link takes: {error:?}\n{batch:#?}");
                }
                tick += u64::from(input.byte() % 70);
                host.begin_step(tick, input.byte() & 1 != 0);
                let context = if input.byte().is_multiple_of(4) { ContextId::Channel } else { ContextId::Page };
                let kind = EventKind::from_number(i32::from(input.byte() % 24));
                host.begin_callback(context, kind, input.int());
            }
            1 => {
                // What the decoder reports of the page's drawings, on the
                // handles the host gave.
                let step = Step {
                    painted: (0..input.byte() % 3).map(|_| number(&mut input, &kept)).collect(),
                    cycle_ended: (0..input.byte() % 3).map(|_| number(&mut input, &kept)).collect(),
                    bounds: (0..input.byte() % 3)
                        .map(|_| Bounds { handle: number(&mut input, &kept), x: input.int(), y: input.int(), width: input.int(), height: input.int() })
                        .collect(),
                    ..Step::default()
                };
                host.regis_reports(&step);
                for context in [ContextId::Channel, ContextId::Page] {
                    let _ = host.regis_take_failure(context);
                }
            }
            2 => {
                // A context closes and opens again; what it held goes.
                let context = if input.byte() & 1 == 0 { ContextId::Channel } else { ContextId::Page };
                host.end_callback();
                host.close(context);
                host.open(opening(context, renderer, input.byte()), 10);
                host.begin_callback(ContextId::Page, None, 0);
                kept.clear();
            }
            _ => {
                let op = operations[usize::from(input.byte()) % operations.len()];
                let count = usize::from(input.byte()) % (MAX_HOST_INTS.min(10) + 1);
                let ints: Vec<i32> = (0..count).map(|_| number(&mut input, &kept)).collect();
                let (form, bytes) = payload_for(op, &mut input, &kept);
                let payload = match form {
                    0 => Payload::None,
                    1 => Payload::Text(&bytes),
                    2 => Payload::Bytes(&bytes),
                    _ => Payload::Ints(&bytes),
                };
                let context = if input.byte() < 32 { 1 } else { 2 };
                if let Reply::Int(handle) = host.call(context, op.number(), &ints, payload)
                    && handle != 0
                    && !kept.contains(&handle)
                {
                    if kept.len() == MAX_KEPT {
                        kept.remove(0);
                    }
                    kept.push(handle);
                }
            }
        }
        assert!(host.state_bytes() <= MAX_HOST_STATE_BYTES, "the host holds {} bytes", host.state_bytes());
        assert!(host.batch_bytes() <= MAX_BATCH_BYTES, "one step's output is {} bytes", host.batch_bytes());
    }
    host.end_callback();
    let batch = host.take_batch(sequence, &[]);
    if let Err(error) = write_to_decoder(&mut Vec::new(), &FromScriptHost::Batch(Box::new(batch.clone()))) {
        panic!("the batch is not a message the link takes: {error:?}\n{batch:#?}");
    }
});
