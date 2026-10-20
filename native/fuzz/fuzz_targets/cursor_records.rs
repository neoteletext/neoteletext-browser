//! Cursor records a decoder passes on, and the trusted core's rules for
//! them (requirement 18.6): any body, on an open input channel.
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext::typing::{Send, Typing};

fuzz_target!(|data: &[u8]| {
    let key = interface_core::canvas::PageKey { channel: 7, page: 0x100 };
    let mut typing = Typing::default();
    typing.input(7, true, false, 0);
    typing.opened(Some(7), Some(key), 0);
    for (now, body) in data.as_chunks::<8>().0.iter().enumerate() {
        for send in typing.cursor(7, *body, now as u64) {
            if let Send::Interface(state) = send {
                // Only what the engine's rules allow reaches the interface.
                if let Some(cursor) = state.cursor {
                    assert!((1..=23).contains(&cursor.row) && cursor.column < 40);
                    assert!(usize::from(cursor.column) + usize::from(cursor.width) <= 40);
                }
            }
        }
    }
});
