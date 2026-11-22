//! The standalone runner's input script reader (script-api.md section
//! 16; `script_run::input`): arbitrary bytes are read or refused, never
//! a panic, and what is read holds within the bounds a step's message
//! has, so that a script of input can make no step the link refuses.
#![no_main]

use libfuzzer_sys::fuzz_target;
use protocol::script::{MAX_EVENT_PAYLOAD_BYTES, Step};
use script_run::input::{Item, parse};
use script_run::limits::{MAX_INPUT_EVENTS, MAX_TICKS};

fuzz_target!(|data: &[u8]| {
    let Ok(script) = parse(data) else { return };
    assert!(script.items.len() <= MAX_INPUT_EVENTS);
    let mut last = 0;
    for (tick, item) in &script.items {
        assert!(*tick >= last && *tick < MAX_TICKS, "the ticks never go down and stay in a run");
        last = *tick;
        // Each item alone is a step the link's own check takes.
        let mut step = Step::default();
        match item {
            Item::Event(event) => {
                assert!(event.payload.len() <= MAX_EVENT_PAYLOAD_BYTES);
                step.events.push(event.clone());
            }
            Item::Painted(handle) => step.painted.push(*handle),
            Item::CycleEnded(handle) => step.cycle_ended.push(*handle),
            Item::Sound(report) => step.sound.push(*report),
            Item::Fault(fault) => step.faults.push(fault.clone()),
            Item::Bounds(bounds) => step.bounds.push(*bounds),
            Item::Hide | Item::Show | Item::Close => {}
        }
        step.check().expect("an input script's item is a step the link takes");
    }
});
