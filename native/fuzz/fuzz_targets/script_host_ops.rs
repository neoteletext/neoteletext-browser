//! The script library's host (script-api.md section 11.4, target 1; the
//! security review's finding S-06, TODO-7272): every host operation with
//! arbitrary whole numbers and payloads, from either context, interleaved
//! with steps, input, contexts closing and opening, and batches taken.
//! Nothing panics; the bytes the host holds for the container stay within
//! `MAX_HOST_STATE_BYTES` and one step's output within `MAX_BATCH_BYTES`;
//! and a batch holds no more than its sections' bounds.
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext_script::host::{Host, Payload};
use neoteletext_script::limits::{MAX_BATCH_BYTES, MAX_HOST_INTS, MAX_HOST_STATE_BYTES, MAX_LOG_CHARS, MAX_LOG_LINES, MAX_SOUND_OPERATIONS_PER_STEP, MAX_TITLE_BYTES, MAX_URL_BYTES};
use neoteletext_script::ops::{EventKind, Op};
use neoteletext_script::{Claims, ContextId, ContextOpen, EventPayload, PointerClaim, Renderer, StepEvent};

/// The most commands one input gives.
const MAX_COMMANDS: usize = 512;
/// The longest payload one command gives: past `MAX_SHARED_VALUE_BYTES`,
/// so that the store's bounds are reached.
const MAX_PAYLOAD: usize = 20_000;
/// Whole numbers at the edges of every operation's ranges.
const EDGES: [i32; 12] = [0, 1, -1, 2, 3, 4, 64, 127, 128, 255, i32::MAX, i32::MIN];

/// Reads the input a piece at a time; past its end every read is zero.
struct Input<'a>(&'a [u8]);

impl Input<'_> {
    fn byte(&mut self) -> u8 {
        let Some((first, rest)) = self.0.split_first() else { return 0 };
        self.0 = rest;
        *first
    }

    fn int(&mut self) -> i32 {
        let choice = self.byte();
        if choice < 128 {
            // A small number, or an edge.
            if choice < 100 { i32::from(choice) } else { EDGES[usize::from(choice) % EDGES.len()] }
        } else {
            i32::from_le_bytes([self.byte(), self.byte(), self.byte(), self.byte()])
        }
    }

    fn bytes(&mut self) -> Vec<u8> {
        let wanted = usize::from(self.byte()) * usize::from(self.byte().max(1));
        let length = wanted.min(MAX_PAYLOAD);
        // A short piece of the input, repeated to the length asked for, so
        // that long payloads do not need long inputs.
        let piece = usize::from(self.byte() % 16) + 1;
        let (seed, rest) = self.0.split_at(piece.min(self.0.len()));
        self.0 = rest;
        if seed.is_empty() { vec![b'x'; length] } else { seed.iter().copied().cycle().take(length).collect() }
    }
}

fn opening(context: ContextId, input: &mut Input<'_>) -> ContextOpen {
    let flags = input.byte();
    ContextOpen {
        context,
        api: 1,
        renderer: Some(Renderer::Teletext),
        page: 0x100,
        subcode: (flags & 1 != 0).then_some(1),
        claims: Claims {
            keys: vec!["Enter".into(), "a".into()],
            text: flags & 2 != 0,
            pointer: if flags & 4 != 0 { PointerClaim::Move } else { PointerClaim::None },
            wheel: flags & 8 != 0,
            tick_divisor: 1,
            network: false,
        },
        language: "en".into(),
        channel_url: "https://example.test/chan/".into(),
        page_url: "https://example.test/chan/100.tti".into(),
        script_url: "https://example.test/chan/page.js".into(),
        channel_title: "Example".into(),
        seed: [flags; 32],
        epoch_seconds: 1_800_000_000,
        zone_minutes: 0,
        sounds: vec![1, 2, 255],
        sound_allowed: flags & 16 != 0,
    }
}

fn context_of(byte: u8) -> ContextId {
    if byte & 1 == 0 { ContextId::Page } else { ContextId::Channel }
}

fuzz_target!(|data: &[u8]| {
    let mut input = Input(data);
    let mut host = Host::new();
    host.open(opening(ContextId::Channel, &mut input), 10);
    host.open(opening(ContextId::Page, &mut input), 10);
    let mut tick = 0_u64;
    host.begin_step(tick, true);
    for _ in 0..MAX_COMMANDS {
        if input.0.is_empty() {
            break;
        }
        match input.byte() % 16 {
            0 => {
                tick += u64::from(input.byte() % 70);
                host.begin_step(tick, input.byte() & 1 != 0);
            }
            1 => {
                let batch = host.take_batch(1, Vec::new());
                assert!(batch.log.len() <= MAX_LOG_LINES && batch.log.iter().all(|line| line.text.chars().count() <= MAX_LOG_CHARS));
                // A step's own operations and its releases are within the
                // bound; stops for a context that ended are the host's own.
                assert!(batch.sound.len() <= MAX_SOUND_OPERATIONS_PER_STEP as usize + 16, "{} sound operations", batch.sound.len());
                assert!(batch.title.as_ref().is_none_or(|title| title.len() <= MAX_TITLE_BYTES));
                assert!(batch.navigate.as_ref().is_none_or(|target| target.len() <= MAX_URL_BYTES));
            }
            2 => {
                let context = context_of(input.byte());
                let kind = EventKind::from_number(i32::from(input.byte() % 24));
                host.begin_callback(context, kind, input.int());
            }
            3 => host.end_callback(),
            4 => {
                let context = context_of(input.byte());
                host.close(context);
                if input.byte() & 1 == 0 {
                    let open = opening(context, &mut input);
                    host.open(open, 10);
                }
            }
            5 => {
                let context = context_of(input.byte());
                let Some(kind) = EventKind::from_number(i32::from(input.byte() % 24)) else { continue };
                let ints = [input.int(), input.int(), input.int(), input.int()];
                let payload = match input.byte() % 4 {
                    0 => EventPayload::None,
                    1 => EventPayload::Text(String::from_utf8_lossy(&input.bytes()).chars().take(64).collect()),
                    2 => EventPayload::Text(if input.byte() & 1 == 0 { "Enter".into() } else { "a".into() }),
                    _ => EventPayload::Ints(vec![input.int(), input.int(), input.int(), input.int()]),
                };
                let _ = host.take_in(context, &StepEvent { kind, ints, payload });
            }
            6 => {
                let context = context_of(input.byte());
                for id in host.due_timers(context) {
                    let _ = host.fire_timer(context, id);
                }
                let _ = host.take_shared_notice();
                let _ = host.count_failure(context, input.byte() & 1 != 0);
            }
            _ => {
                // A host call: a context, an operation, whole numbers, a payload.
                let context = match input.byte() % 8 {
                    0 => 0,
                    1 => 3,
                    other => i32::from(other % 2) + 1,
                };
                let choice = input.byte();
                let op = if choice < 240 { Op::ALL[usize::from(choice) % Op::ALL.len()].1.number() } else { input.int() };
                let count = usize::from(input.byte()) % (MAX_HOST_INTS + 2);
                let ints: Vec<i32> = (0..count.min(6)).map(|_| input.int()).chain(std::iter::repeat_n(0, count.saturating_sub(6))).collect();
                let bytes = input.bytes();
                let payload = match input.byte() % 5 {
                    0 => Payload::None,
                    1 | 2 => Payload::Text(&bytes),
                    3 => Payload::Bytes(&bytes),
                    _ => Payload::Ints(&bytes),
                };
                let _ = host.call(context, op, &ints, payload);
            }
        }
        assert!(host.state_bytes() <= MAX_HOST_STATE_BYTES, "the host holds {} bytes", host.state_bytes());
        assert!(host.batch_bytes() <= MAX_BATCH_BYTES, "one step's output is {} bytes", host.batch_bytes());
    }
});
