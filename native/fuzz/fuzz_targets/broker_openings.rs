//! What the broker reads from a second start (DEC-1551 item 7), which may be
//! any program on this computer that reaches the channel.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut stream = data;
    core::hint::black_box(confine::broker::read_opening(&mut stream)).ok();
});
