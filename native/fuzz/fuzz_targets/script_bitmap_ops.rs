//! The sixel bitmap's host operations (script-api.md section 5.7 and
//! operations 130 to 138 of section 10.2; work package T3; section 11.4
//! asks a target of every reader, and a script's arguments are outside
//! input): every bitmap operation with arbitrary whole numbers, sixels and
//! text, from either context and at every level, interleaved with the
//! rest of the teletext API that shares the page with it (the page plane,
//! the overlay, the controls, the Colour Map and the settings), with
//! steps and with batches taken. The engine's own target `bitmap_present`
//! gives the converter an arbitrary bitmap and frame; this one gives the
//! host that keeps the two buffers an arbitrary script.
//!
//! Nothing panics; `get` gives a colour a sixel may have or nothing;
//! `present` gives no more cells than the bitmap has; the page's local
//! enhancement data never passes `LOCAL_TRIPLETS`, whatever the bitmap and
//! the overlay ask for together; every row sent is one of rows 1 to 23
//! with seven-bit codes; what the host holds stays within
//! `MAX_HOST_STATE_BYTES`; and every batch is a message the link takes.
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext_script::host::{Host, Payload, Reply};
use neoteletext_script::limits::{MAX_BATCH_BYTES, MAX_HOST_STATE_BYTES};
use neoteletext_script::ops::Op;
use neoteletext_script::teletext::bitmap::{CELLS, PRESENT_LOSSY_FACTOR};
use neoteletext_script::teletext::limits::LOCAL_TRIPLETS;
use protocol::script::batch::{FIRST_SCRIPT_ROW, LAST_SCRIPT_ROW, MAX_ROW_CODE};
use protocol::script::{ContextFlags, ContextId, ContextOpen, FromScriptHost, Level, Renderer, write_to_decoder};

/// The most commands one input gives.
const MAX_COMMANDS: usize = 512;
/// The longest payload one command gives: a whole bitmap of sixels, 80 by
/// 69, and some past it.
const MAX_PAYLOAD: usize = 6000;
/// Whole numbers at the edges of the bitmap's ranges: its sides, its
/// rows, its colours, the coordinate bound, the "not given" −1, a byte's
/// end and the ends of the whole numbers.
const EDGES: [i32; 24] = [0, 1, -1, 2, 7, 8, 15, 16, 23, 24, 31, 32, 39, 40, 68, 69, 79, 80, 255, 256, 1024, 1025, i32::MAX, i32::MIN];
/// Text for `bitmap.text` and the overlay: the page's own G0 characters,
/// a mark, a G2 character, another script and what no set shows.
const TEXTS: [&str; 6] = ["SCORE 000", "caf\u{e9}", "\u{a3}5 #", "\u{3b1}\u{3b2}", "\u{2603}", ""];
/// The languages a context's opening names.
const LANGUAGES: [&str; 3] = ["en", "de", "no-such"];
/// The page's own size in sixels, for a whole-page blit.
const WHOLE_PAGE: [i32; 5] = [0, 0, 80, 69, 255];

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
            0..=127 => i32::from(choice % 90) - 4,
            128..=239 => EDGES[usize::from(choice) % EDGES.len()],
            _ => i32::from_le_bytes([self.byte(), self.byte(), self.byte(), self.byte()]),
        }
    }

    /// Bytes for a blit: mostly colours a sixel may have, so that whole
    /// frames are drawn and presented, and now and then any byte.
    fn sixels(&mut self, length: usize) -> Vec<u8> {
        let wild = self.byte().is_multiple_of(8);
        let piece = usize::from(self.byte() % 16) + 1;
        let (seed, rest) = self.0.split_at(piece.min(self.0.len()));
        self.0 = rest;
        let colour = |byte: u8| {
            if wild {
                byte
            } else if byte & 0x80 == 0 {
                byte % 8
            } else {
                16 + byte % 16
            }
        };
        if seed.is_empty() { vec![0; length] } else { seed.iter().copied().cycle().take(length).map(colour).collect() }
    }
}

fn opening(context: ContextId, input: &mut Bytes<'_>) -> ContextOpen {
    let choice = input.byte();
    ContextOpen {
        context,
        api: 1,
        renderer: if choice & 0x38 == 0x38 { Renderer::Regis } else { Renderer::Teletext },
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

/// The bitmap's operations, and the teletext operations that share the
/// page with it.
fn operations() -> (Vec<Op>, Vec<Op>) {
    let all = || Op::ALL.iter().map(|(_, op)| *op);
    (all().filter(|op| (130..140).contains(&op.number())).collect(), all().filter(|op| (100..120).contains(&op.number()) || *op == Op::BUDGET).collect())
}

fuzz_target!(|data: &[u8]| {
    let mut input = Bytes(data);
    let (bitmap, others) = operations();
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
        match input.byte() % 32 {
            0 => {
                tick += u64::from(input.byte() % 70);
                host.end_callback();
                host.begin_step(tick, true);
                host.begin_callback(ContextId::Page, None, 0);
            }
            1..=3 => {
                let batch = host.take_batch(1, &[]);
                let triplets = batch.teletext_enhancement.as_ref().map_or(0, Vec::len);
                assert!(triplets <= LOCAL_TRIPLETS, "{triplets} triplets of local enhancement data");
                for row in batch.teletext_rows.iter().flatten() {
                    assert!((FIRST_SCRIPT_ROW..=LAST_SCRIPT_ROW).contains(&row.row) && row.codes.iter().all(|code| *code <= MAX_ROW_CODE), "{row:?}");
                }
                let mut written = Vec::new();
                if let Err(error) = write_to_decoder(&mut written, &FromScriptHost::Batch(Box::new(batch.clone()))) {
                    panic!("the batch is not a message the link takes: {error:?}\n{batch:#?}");
                }
            }
            4 => {
                let context = if input.byte() & 1 == 0 { ContextId::Page } else { ContextId::Channel };
                host.close(context);
                let open = opening(context, &mut input);
                host.open(open, 10);
            }
            // A whole frame drawn and shown, as an animation does.
            5..=8 => {
                let frame = input.sixels(80 * 69);
                let _ = host.call(2, Op::BM_BLIT.number(), &WHOLE_PAGE, Payload::Bytes(&frame));
                present(&mut host, 2);
            }
            choice => {
                let context = if input.byte().is_multiple_of(8) { 1 } else { 2 };
                let from = if choice < 24 { &bitmap } else { &others };
                let op = from[usize::from(input.byte()) % from.len()];
                if op == Op::BM_PRESENT && !input.byte().is_multiple_of(4) {
                    present(&mut host, context);
                    continue;
                }
                let count = usize::from(input.byte() % 9);
                let ints: Vec<i32> = (0..count).map(|_| input.int()).collect();
                let kind = input.byte();
                let bytes = match kind % 4 {
                    0 => TEXTS[usize::from(kind / 4) % TEXTS.len()].as_bytes().to_vec(),
                    // A blit's data of exactly its width by its height.
                    1 => {
                        let wanted = ints.get(2).zip(ints.get(3)).map_or(0, |(width, height)| i64::from(*width).saturating_mul(i64::from(*height)));
                        input.sixels(usize::try_from(wanted).unwrap_or(0).min(MAX_PAYLOAD))
                    }
                    _ => {
                        let length = usize::from(input.byte()) * 24;
                        input.sixels(length.min(MAX_PAYLOAD))
                    }
                };
                let payload = match kind % 4 {
                    0 => Payload::Text(&bytes),
                    1 | 2 => Payload::Bytes(&bytes),
                    _ => Payload::None,
                };
                let reply = host.call(context, op.number(), &ints, payload);
                if let (Op::BM_GET, Reply::Int(colour)) = (op, &reply) {
                    assert!((0..8).contains(colour) || (16..32).contains(colour), "get gave the colour {colour}");
                }
            }
        }
        assert!(host.state_bytes() <= MAX_HOST_STATE_BYTES, "the host holds {} bytes", host.state_bytes());
        assert!(host.batch_bytes() <= MAX_BATCH_BYTES, "one step's output is {} bytes", host.batch_bytes());
    }
});

/// `present()`: when it answers with a number, the cells it counts as
/// changed and as lossy are each no more than the bitmap's cells.
fn present(host: &mut Host, context: i32) {
    if let Reply::Int(packed) = host.call(context, Op::BM_PRESENT.number(), &[], Payload::None) {
        let (cells, lossy) = (packed % PRESENT_LOSSY_FACTOR, packed / PRESENT_LOSSY_FACTOR);
        assert!(
            packed >= 0 && usize::try_from(cells).is_ok_and(|cells| cells <= CELLS) && usize::try_from(lossy).is_ok_and(|lossy| lossy <= CELLS),
            "present gave {packed}"
        );
    }
}
