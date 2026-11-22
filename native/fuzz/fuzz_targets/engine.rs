//! A channel's bytes through the decoder process's engine: format
//! detection, decoding, the page store, titles and rendering.
#![no_main]

use std::cell::RefCell;

use decoder::Engine;
use libfuzzer_sys::fuzz_target;

thread_local! {
    // One engine for the whole run: each engine leaks its page store, one
    // per process by design, so an engine per input would exhaust memory.
    // Pages from earlier inputs stay in the store, so a crash may need
    // the inputs before it to reproduce. It is made before the first
    // input and never dropped, so that the leak detector, which looks
    // after an input that allocated more than it freed and again as the
    // process ends, finds the store and the sound arena it leaks still
    // reachable through it.
    static ENGINE: &'static RefCell<Engine> = Box::leak(Box::new(RefCell::new(Engine::new())));
}

fuzz_target!(init: { ENGINE.with(|_| ()); }, |data: &[u8]| {
    ENGINE.with(|engine| {
        let mut held = engine.borrow_mut();
        let engine = &mut *held;
        let mut arrived = engine.feed(data);
        arrived.extend(engine.finish());
        core::hint::black_box(engine.take_updated());
        core::hint::black_box(engine.export());
        for (page, _) in arrived {
            core::hint::black_box(engine.render(page));
        }
    });
});
