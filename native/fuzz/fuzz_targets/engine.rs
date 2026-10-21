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
    // the inputs before it to reproduce.
    static ENGINE: RefCell<Engine> = RefCell::new(Engine::new());
}

fuzz_target!(|data: &[u8]| {
    ENGINE.with_borrow_mut(|engine| {
        let mut arrived = engine.feed(data);
        arrived.extend(engine.finish());
        core::hint::black_box(engine.take_updated());
        core::hint::black_box(engine.export());
        for (page, _) in arrived {
            core::hint::black_box(engine.render(page));
        }
    });
});
