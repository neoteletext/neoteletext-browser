//! The teletext API's host operations (script-api.md section 11.4, target
//! 1 as work package T1 extends it; sections 5.1 to 5.6 and 10.1): every
//! teletext operation with arbitrary whole numbers, text and codes, from
//! either context, interleaved with steps, scopes and batches taken.
//! `script_host_ops` reaches these operations among all the host's; this
//! target gives them the whole input, so that the page plane, the overlay
//! and the text encoder are driven deep. Nothing panics; the page's local
//! enhancement data never passes `LOCAL_TRIPLETS`; every row sent is one
//! of rows 1 to 23 with seven-bit codes; what the host holds stays within
//! `MAX_HOST_STATE_BYTES`; and every batch is a message the link takes.
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext_script::host::{Host, Payload};
use neoteletext_script::limits::{MAX_BATCH_BYTES, MAX_HOST_STATE_BYTES};
use neoteletext_script::ops::Op;
use neoteletext_script::teletext::limits::{LOCAL_TRIPLETS, MAX_CONTROLS};
use protocol::script::batch::{FIRST_SCRIPT_ROW, LAST_SCRIPT_ROW, MAX_ROW_CODE};
use protocol::script::{ContextFlags, ContextId, ContextOpen, FromScriptHost, Level, Renderer, write_to_decoder};

/// The most commands one input gives.
const MAX_COMMANDS: usize = 512;
/// The longest payload one command gives: past a whole plane of codes.
const MAX_PAYLOAD: usize = 1200;
/// Whole numbers at the edges of the teletext operations' ranges: rows,
/// columns, colours, entries, the "not given" −1 and the flags' ends.
const EDGES: [i32; 16] = [0, 1, -1, 2, 7, 8, 16, 22, 23, 24, 31, 32, 39, 40, i32::MAX, i32::MIN];
/// Text the encoder treats differently: marks, a national option, G2,
/// right-to-left, a set of NeoTeletext's own, and what no set shows.
const TEXTS: [&str; 8] =
    ["HARBOUR NEWS", "caf\u{e9} \u{a3}5", "Gr\u{fc}\u{df}e", "\u{3b1}\u{3b2}\u{3b3}", "\u{5e9}\u{5dc}\u{5d5}\u{5dd} abc", "\u{10d0}\u{10d1}", "\u{2603}", ""];
/// The languages a context's opening names.
const LANGUAGES: [&str; 4] = ["en", "de", "ar", "no-such"];

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
        match choice {
            0..=127 => i32::from(choice % 48) - 2,
            128..=239 => EDGES[usize::from(choice) % EDGES.len()],
            _ => i32::from_le_bytes([self.byte(), self.byte(), self.byte(), self.byte()]),
        }
    }

    fn bytes(&mut self) -> Vec<u8> {
        let length = (usize::from(self.byte()) * usize::from(self.byte().max(1)) / 16).min(MAX_PAYLOAD);
        let piece = usize::from(self.byte() % 8) + 1;
        let (seed, rest) = self.0.split_at(piece.min(self.0.len()));
        self.0 = rest;
        if seed.is_empty() { vec![b' '; length] } else { seed.iter().copied().cycle().take(length).collect() }
    }
}

fn opening(context: ContextId, input: &mut Bytes<'_>) -> ContextOpen {
    let choice = input.byte();
    ContextOpen {
        context,
        api: 1,
        renderer: if choice & 0x30 == 0x30 { Renderer::Regis } else { Renderer::Teletext },
        level: [Level::One, Level::OneFive, Level::TwoFive, Level::ThreeFive][usize::from(choice >> 6)],
        page: 0x100,
        subcode: 0,
        flags: ContextFlags::default(),
        tick_divisor: 1,
        language: LANGUAGES[usize::from(choice) % LANGUAGES.len()].into(),
        channel_url: "https://example.test/chan/".into(),
        page_url: "https://example.test/chan/100.tti".into(),
        script_url: "https://example.test/chan/page.js".into(),
        channel_title: "Example".into(),
        seed: [choice; 32],
        epoch_seconds: 1_800_000_000,
        zone_minutes: 0,
        keys: Vec::new(),
        sounds: Vec::new(),
        source_bytes: 10,
    }
}

/// The teletext operations, and the scope operations their controls
/// belong to.
fn operations() -> Vec<Op> {
    Op::ALL
        .iter()
        .map(|(_, op)| *op)
        .filter(|op| (100..120).contains(&op.number()) || matches!(op, Op::SCOPE_BEGIN | Op::SCOPE_LEAVE | Op::SCOPE_END | Op::LISTEN | Op::BUDGET))
        .collect()
}

fuzz_target!(|data: &[u8]| {
    let mut input = Bytes(data);
    let operations = operations();
    let mut host = Host::new();
    host.open(opening(ContextId::Channel, &mut input), 10);
    host.open(opening(ContextId::Page, &mut input), 10);
    let mut tick = 0_u64;
    host.begin_step(tick, true);
    host.begin_callback(ContextId::Page, None, 0);
    for _ in 0..MAX_COMMANDS {
        if input.0.is_empty() {
            break;
        }
        match input.byte() % 16 {
            0 => {
                tick += u64::from(input.byte() % 70);
                host.end_callback();
                host.begin_step(tick, true);
                host.begin_callback(ContextId::Page, None, 0);
            }
            1 | 2 => {
                let batch = host.take_batch(1, &[]);
                let triplets = batch.teletext_enhancement.as_ref().map_or(0, Vec::len);
                assert!(triplets <= LOCAL_TRIPLETS, "{triplets} triplets of local enhancement data");
                for row in batch.teletext_rows.iter().flatten() {
                    assert!((FIRST_SCRIPT_ROW..=LAST_SCRIPT_ROW).contains(&row.row) && row.codes.iter().all(|code| *code <= MAX_ROW_CODE), "{row:?}");
                }
                assert!(batch.labels.as_ref().map_or(0, Vec::len) <= MAX_CONTROLS);
                let mut written = Vec::new();
                if let Err(error) = write_to_decoder(&mut written, &FromScriptHost::Batch(Box::new(batch.clone()))) {
                    panic!("the batch is not a message the link takes: {error:?}\n{batch:#?}");
                }
            }
            3 => {
                let context = if input.byte() & 1 == 0 { ContextId::Page } else { ContextId::Channel };
                host.close(context);
                let open = opening(context, &mut input);
                host.open(open, 10);
            }
            _ => {
                let context = if input.byte() % 8 == 0 { 1 } else { 2 };
                let op = operations[usize::from(input.byte()) % operations.len()].number();
                let count = usize::from(input.byte() % 9);
                let ints: Vec<i32> = (0..count).map(|_| input.int()).collect();
                let choice = input.byte();
                let bytes = if choice % 4 == 0 { TEXTS[usize::from(choice / 4) % TEXTS.len()].as_bytes().to_vec() } else { input.bytes() };
                let payload = match choice % 4 {
                    0 | 1 => Payload::Text(&bytes),
                    2 => Payload::Bytes(&bytes),
                    _ => Payload::None,
                };
                let _ = host.call(context, op, &ints, payload);
            }
        }
        assert!(host.state_bytes() <= MAX_HOST_STATE_BYTES, "the host holds {} bytes", host.state_bytes());
        assert!(host.batch_bytes() <= MAX_BATCH_BYTES, "one step's output is {} bytes", host.batch_bytes());
    }
});
