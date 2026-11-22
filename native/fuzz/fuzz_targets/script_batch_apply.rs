//! A script host's batches applied to the open page by the decoder
//! (script-api.md section 11.2 and section 11.4, target 6;
//! `decoder::Engine::apply_script`). The script host runs a channel's
//! script and is untrusted, so the batches are of two kinds: frames of
//! arbitrary bytes, as a compromised script host could write them, read
//! by the link's own reader (`read_to_decoder`); and the batches a real
//! host makes from an arbitrary script's host calls on a teletext, a
//! ReGIS or a NAPLPS page. Each is applied to an open page of any
//! renderer, then the page is rendered and picked at a pixel. Whatever
//! the batches, nothing panics, and the only pages that arrive are the
//! open page and the script's Object page.
#![no_main]

use std::cell::RefCell;

use decoder::{Engine, ScriptPage};
use libfuzzer_sys::fuzz_target;
use neoteletext_core::limits::{FIRST_PAGE_ADDRESS, LAST_PAGE_ADDRESS};
use neoteletext_script::host::{Host, Payload};
use neoteletext_script::limits::MAX_HOST_INTS;
use neoteletext_script::ops::Op;
use protocol::script::sprites::SCRIPT_OBJECT_PAGE;
use protocol::script::{ContextFlags, ContextId, ContextOpen, FromScriptHost, Level, Renderer, read_to_decoder};

thread_local! {
    // One engine for the whole run: each engine leaks its page store, one
    // per process by design (the `engine` target says the same). It is
    // made before the first input (the `init` below), so that the leak
    // detector, which counts what each input allocates and does not free,
    // never takes the store for an input's leak; and it is never dropped,
    // so that the store stays reachable through it when the detector
    // looks again as the process ends.
    static ENGINE: &'static RefCell<Engine> = Box::leak(Box::new(RefCell::new(Engine::new())));
}

/// The most host calls one input makes, and the most batches it applies.
const MAX_CALLS: usize = 256;
const MAX_BATCHES: usize = 16;
/// The longest payload one call gives.
const MAX_PAYLOAD: usize = 8192;
/// Whole numbers at the edges of the operations' ranges.
const EDGES: [i32; 14] = [0, 1, -1, 2, 23, 24, 39, 40, 255, 256, 1024, 65_535, i32::MAX, i32::MIN];
/// The pages a script may open: the first and last there are, the last
/// of a magazine, the Object page itself, and page FF, which is no page.
const PAGES: [u16; 5] = [FIRST_PAGE_ADDRESS, 0x1FE, LAST_PAGE_ADDRESS, SCRIPT_OBJECT_PAGE, 0x1FF];
/// The largest pixel picked, past every renderer's picture.
const MAX_PIXEL: u16 = 1100;

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
            choice @ 0..=99 => i32::from(choice),
            choice @ 100..=199 => EDGES[usize::from(choice) % EDGES.len()],
            _ => i32::from_le_bytes([self.byte(), self.byte(), self.byte(), self.byte()]),
        }
    }

    fn bytes(&mut self) -> Vec<u8> {
        let length = (usize::from(self.byte()) * usize::from(self.byte().max(1))).min(MAX_PAYLOAD);
        let piece = usize::from(self.byte() % 16) + 1;
        let (seed, rest) = self.0.split_at(piece.min(self.0.len()));
        self.0 = rest;
        if seed.is_empty() { vec![b' '; length] } else { seed.iter().copied().cycle().take(length).collect() }
    }
}

fn renderer_of(byte: u8) -> Renderer {
    [Renderer::Teletext, Renderer::Regis, Renderer::Naplps][usize::from(byte % 3)]
}

fn opening(renderer: Renderer, page: u16, sub: u16) -> ContextOpen {
    ContextOpen {
        context: ContextId::Page,
        api: 1,
        renderer,
        level: Level::TwoFive,
        page,
        subcode: sub,
        flags: ContextFlags::default(),
        tick_divisor: 1,
        language: "en".into(),
        channel_url: "https://example.test/chan/".into(),
        page_url: "https://example.test/chan/100.tti".into(),
        script_url: "https://example.test/chan/page.js".into(),
        channel_title: "Example".into(),
        seed: [7; 32],
        epoch_seconds: 1_800_000_000,
        zone_minutes: 0,
        keys: Vec::new(),
        sounds: vec![1],
        source_bytes: 10,
    }
}

/// The operations a page of `renderer` has, with the shared core's.
fn operations(renderer: Renderer) -> Vec<Op> {
    let own = match renderer {
        Renderer::Teletext => 100..140,
        Renderer::Regis => 200..216,
        Renderer::Naplps => 300..313,
    };
    Op::ALL.iter().map(|(_, op)| *op).filter(|op| own.contains(&op.number()) || (60..68).contains(&op.number()) || op.number() < 30).collect()
}

/// Applies `batch` to the open page and looks at what it made.
fn apply(engine: &mut Engine, at: ScriptPage, batch: &protocol::script::Batch, designate: bool, pixel: (u16, u16)) {
    if let Ok(arrived) = engine.apply_script(at, batch, designate) {
        for (page, _) in &arrived {
            assert!(*page == at.page || *page == SCRIPT_OBJECT_PAGE, "a batch for page {:03X} made page {page:03X} arrive", at.page);
            core::hint::black_box(engine.render(*page));
        }
    }
    core::hint::black_box(engine.script_hit(at, pixel.0, pixel.1));
    core::hint::black_box(engine.script_renderer(at.page, at.sub));
}

fuzz_target!(init: { ENGINE.with(|_| ()); }, |data: &[u8]| {
    let mut input = Bytes(data);
    let mode = input.byte();
    let renderer = renderer_of(input.byte());
    let page = PAGES[usize::from(input.byte()) % PAGES.len()];
    let sub = u16::from(input.byte() % 4);
    let at = ScriptPage { page, sub, renderer, designation: if input.byte() & 1 == 0 { None } else { Some(input.byte() % 0x58) } };
    let pixel = (u16::from_le_bytes([input.byte(), input.byte()]) % MAX_PIXEL, u16::from_le_bytes([input.byte(), input.byte()]) % MAX_PIXEL);
    ENGINE.with(|engine| {
        let mut held = engine.borrow_mut();
        let engine = &mut *held;
        engine.script_begins_again(at);
        for (page, _) in engine.script_blank_page(at) {
            assert!(page == at.page, "opening page {:03X} blank made page {page:03X} arrive", at.page);
        }
        if mode & 1 == 0 {
            // A compromised script host's frames, as the link reads them.
            let mut frames = input.0;
            let mut designate = true;
            for _ in 0..MAX_BATCHES {
                let Ok(Some(message)) = read_to_decoder(&mut frames) else { break };
                if let FromScriptHost::Batch(batch) = message {
                    apply(engine, at, &batch, designate, pixel);
                    designate = false;
                }
            }
        } else {
            // A real host's batches from an arbitrary script's calls.
            let operations = operations(renderer);
            let mut host = Host::new();
            host.open(opening(renderer, page, sub), 10);
            let mut tick = 0_u64;
            host.begin_step(tick, true);
            host.begin_callback(ContextId::Page, None, 0);
            let mut handles: Vec<i32> = Vec::new();
            let mut sequence = 0_u32;
            for _ in 0..MAX_CALLS {
                if input.0.is_empty() {
                    break;
                }
                let choice = input.byte();
                if choice < 24 {
                    host.end_callback();
                    let batch = host.take_batch(sequence, &[]);
                    apply(engine, at, &batch, sequence == 0, pixel);
                    sequence = sequence.saturating_add(1);
                    tick += u64::from(input.byte() % 8) + 1;
                    host.begin_step(tick, true);
                    host.begin_callback(ContextId::Page, None, 0);
                    continue;
                }
                let op = operations[usize::from(input.byte()) % operations.len()];
                let count = usize::from(input.byte()) % (MAX_HOST_INTS.min(12) + 1);
                // A handle the host gave earlier, as a script would pass
                // it back, or any number.
                let ints: Vec<i32> = (0..count)
                    .map(|_| if !handles.is_empty() && input.byte() < 96 { handles[usize::from(input.byte()) % handles.len()] } else { input.int() })
                    .collect();
                let bytes = input.bytes();
                let payload = match input.byte() % 4 {
                    0 => Payload::None,
                    1 => Payload::Text(&bytes),
                    2 => Payload::Bytes(&bytes),
                    _ => Payload::Ints(&bytes),
                };
                if let neoteletext_script::host::Reply::Int(handle) = host.call(2, op.number(), &ints, payload)
                    && handle > 0
                    && handles.len() < 64
                {
                    handles.push(handle);
                }
            }
            host.end_callback();
            let batch = host.take_batch(sequence, &[]);
            apply(engine, at, &batch, sequence == 0, pixel);
        }
    });
});
