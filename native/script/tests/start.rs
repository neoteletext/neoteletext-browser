//! Where a context's start goes (rdm TODO-7386): a measurement, which
//! prints its numbers and asserts nothing. Run by hand, in a release
//! build and in a debug build, on a machine with processors to spare:
//!   cargo test --release -p neoteletext-script --test start -- --ignored --nocapture
mod common;

use std::time::Duration;

use common::{container, opening, timed};
use neoteletext_script::limits::START_TIME_LIMIT;
use neoteletext_script::ops::{Parts, prelude_of};
use protocol::script::ContextId;

/// The parts of the prelude as the crate's sources hold them.
const PARTS: &[(&str, &str)] = &[
    ("core", include_str!("../src/prelude.js")),
    ("NAPLPS", include_str!("../src/host/naplps/prelude.js")),
    ("bitmap", include_str!("../src/teletext/bitmap.js")),
    ("teletext", include_str!("../src/teletext.js")),
    ("sprites", include_str!("../src/teletext_sprites.js")),
    ("ReGIS", include_str!("../src/regis.js")),
    ("streams polyfill", include_str!("../src/web/streams/ponyfill.js")),
    ("web: encoding, URL", include_str!("../src/web/encoding_url.js")),
    ("web: fetch", include_str!("../src/web/fetch.js")),
    ("web: sockets", include_str!("../src/web/sockets.js")),
];
/// The runs each figure is the least of.
const RUNS: usize = 7;

/// The least time, of several runs, that opening a page context with
/// `source` and the parts named takes, less the thread's waits for a
/// processor.
fn opening_takes(source: &str, parts: Parts) -> Duration {
    (0..RUNS)
        .map(|_| {
            let mut container = container();
            let mut open = opening(ContextId::Page);
            open.source_bytes = u32::try_from(source.len()).unwrap();
            // Its bytecode first, as the engine keeps it.
            common::engine().prelude(parts).unwrap();
            timed(|| container.open_with_parts(open, source.as_bytes(), parts).unwrap()).1.running
        })
        .min()
        .unwrap()
}

#[test]
#[ignore = "a measurement that prints its numbers and asserts nothing; run by hand on a quiet machine"]
fn where_a_contexts_start_goes() {
    let engine_began = std::time::Instant::now();
    let _ = common::engine();
    println!("the engine compiled in {:?}", engine_began.elapsed());
    println!("a container starts in {:?}", container().costs().container);
    // Once for the engine, and no part of any context's start.
    let (bytecode, took) = timed(|| common::engine().prelude(Parts::ALL).unwrap());
    println!("compiling the prelude once takes {:?} ({} bytes of source, {} of bytecode)", took.running, prelude_of(Parts::ALL).len(), bytecode.len());
    let core = opening_takes(";", Parts::CORE);
    println!("a context with an empty script and the shared core alone opens in {core:?} ({} bytes of prelude)", prelude_of(Parts::CORE).len());
    for (name, parts) in [
        ("teletext, bitmap, sprites", Parts { teletext: true, ..Parts::CORE }),
        ("ReGIS", Parts { regis: true, ..Parts::CORE }),
        ("NAPLPS", Parts { naplps: true, ..Parts::CORE }),
        ("web with the streams", Parts { web: true, ..Parts::CORE }),
    ] {
        let with = opening_takes(";", parts);
        println!("  {name:28} adds {:>12.3?} ({} bytes)", with.saturating_sub(core), prelude_of(parts).len() - prelude_of(Parts::CORE).len());
    }
    let all = opening_takes(";", Parts::ALL);
    println!(
        "with every part, as the browser opens it, it opens in {all:?}: {:.1}% of the start limit",
        all.as_secs_f64() * 100.0 / START_TIME_LIMIT.as_secs_f64()
    );
    // What compiling alone costs: each source held in a function that is
    // never called, as the script of a context with the core alone.
    for (name, part) in PARTS {
        let took = opening_takes(&format!("function never() {{\n{part}\n}}"), Parts::CORE).saturating_sub(core);
        println!("  compiling {name:20} {:>7} bytes {took:>12.3?}", part.len());
    }
}
