//! The script library's host (script-api.md section 11.4, target 1; the
//! security review's finding S-06, TODO-7272): every host operation with
//! arbitrary whole numbers and payloads, from either context, interleaved
//! with steps, input, contexts closing and opening, and batches taken.
//! Nothing panics; the bytes the host holds for the container stay within
//! `MAX_HOST_STATE_BYTES` and one step's output within `MAX_BATCH_BYTES`;
//! and a batch holds no more than its sections' bounds.
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext_script::host::{Host, Input as HostInput, InputPayload, Payload};
use neoteletext_script::limits::{MAX_BATCH_BYTES, MAX_HOST_INTS, MAX_HOST_STATE_BYTES};
use neoteletext_script::ops::{EventKind, Op};
use protocol::script::{Bounds, ContextFlags, ContextId, ContextOpen, Event, EventKind as Carried, FromScriptHost, Level, Renderer, Step, write_to_decoder};

/// The most commands one input gives.
const MAX_COMMANDS: usize = 512;
/// The longest payload one command gives: past `MAX_SHARED_VALUE_BYTES`,
/// so that the store's bounds are reached.
const MAX_PAYLOAD: usize = 20_000;
/// Whole numbers at the edges of every operation's ranges.
const EDGES: [i32; 12] = [0, 1, -1, 2, 3, 4, 64, 127, 128, 255, i32::MAX, i32::MIN];

/// Reads the input a piece at a time; past its end every read is zero.
struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
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

fn opening(context: ContextId, input: &mut Bytes<'_>) -> ContextOpen {
    let flags = input.byte();
    ContextOpen {
        context,
        api: 1,
        // Bits 6 and 7 choose the page's renderer, so that each renderer's
        // operations are reached: a ReGIS page as often as not (operations
        // 200 to 214, work package R1), a NAPLPS page (300 to 312, work
        // packages N1 and N2) or a teletext page a quarter of the time each.
        renderer: match flags >> 6 {
            3 => Renderer::Naplps,
            1 => Renderer::Teletext,
            _ => Renderer::Regis,
        },
        level: Level::TwoFive,
        page: 0x100,
        subcode: u16::from(flags & 1),
        flags: ContextFlags {
            pointer_press: flags & 4 != 0,
            pointer_move: flags & 32 != 0,
            wheel: flags & 8 != 0,
            text: flags & 2 != 0,
            // With it, the web's operations ask (work package W2).
            network: flags & 64 != 0,
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
        sounds: vec![1, 2, 255],
        source_bytes: 10,
    }
}

fn context_of(byte: u8) -> ContextId {
    if byte & 1 == 0 { ContextId::Page } else { ContextId::Channel }
}

fuzz_target!(|data: &[u8]| {
    let mut input = Bytes(data);
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
                // Whatever the calls were, what leaves is a message the
                // link takes whole: every section within its bounds.
                let batch = host.take_batch(1, &[]);
                let mut written = Vec::new();
                if let Err(error) = write_to_decoder(&mut written, &FromScriptHost::Batch(Box::new(batch.clone()))) {
                    panic!("the batch is not a message the link takes: {error:?}\n{batch:#?}");
                }
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
                    0 => InputPayload::None,
                    1 => InputPayload::Text(String::from_utf8_lossy(&input.bytes()).chars().take(64).collect()),
                    2 => InputPayload::Text(if input.byte() & 1 == 0 { "Enter".into() } else { "a".into() }),
                    _ => InputPayload::Ints(vec![input.int(), input.int(), input.int(), input.int()]),
                };
                let _ = host.take_in(context, &HostInput { kind, ints, payload });
            }
            6 => {
                let context = context_of(input.byte());
                for id in host.due_timers(context) {
                    let _ = host.fire_timer(context, id);
                }
                let _ = host.take_shared_notice();
                let _ = host.count_failure(context, input.byte() & 1 != 0);
            }
            7 => {
                // What a step reports of the page's drawings: painted,
                // cycles ended, bounds, and the page left.
                let left = input.byte() % 8 == 0;
                let step = Step {
                    events: if left { vec![Event { kind: Carried::PageHide, numbers: [0x100, 0, 0, 0], payload: Vec::new() }] } else { Vec::new() },
                    painted: vec![input.int()],
                    cycle_ended: vec![input.int(), input.int()],
                    bounds: (0..input.byte() % 4)
                        .map(|_| Bounds { handle: input.int(), x: input.int(), y: input.int(), width: input.int(), height: input.int() })
                        .collect(),
                    ..Step::default()
                };
                host.regis_reports(&step);
                for context in [ContextId::Channel, ContextId::Page] {
                    let _ = host.regis_take_failure(context);
                }
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
