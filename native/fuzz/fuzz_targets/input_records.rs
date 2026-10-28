//! The channel's records for its input that a decoder passes on, and the
//! trusted core's rules for them (REQ-INT-110, REQ-INT-111, REQ-INT-134):
//! any cursor body and any control body, on an open input channel that was
//! welcomed.
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext::typing::{Send, Typing};
use neoteletext_input::control::MAX_BODY;
use neoteletext_input::cursor::{BLOCK, BODY};

/// WELCOME, version 2, with every capability NeoTeletext offers but the
/// agent: pages, the cursor, echo and every mode, 459 007, whose varint
/// is FF 81 1C (INPUT.md section 5.2).
const WELCOME: [u8; 6] = [0xFF, 0x01, 0x02, 0xFF, 0x81, 0x1C];

fuzz_target!(|data: &[u8]| {
    let key = interface_core::canvas::PageKey { channel: 7, page: 0x100 };
    let mut typing = Typing::default();
    typing.input(7, true, false, 0);
    typing.opened(Some(7), Some(key), 0);
    typing.control(7, &WELCOME, 0);
    let mut rest = data;
    let mut now = 0_u64;
    while let Some((&kind, after)) = rest.split_first() {
        now += 1;
        let sends = if kind & 1 == 0 {
            let Some((body, after)) = after.split_first_chunk::<BODY>() else { return };
            rest = after;
            typing.cursor(7, *body)
        } else {
            let length = usize::from(kind >> 1).min(MAX_BODY).min(after.len());
            let (body, after) = after.split_at(length);
            rest = after;
            typing.control(7, body, now)
        };
        for send in sends {
            if let Send::Interface(state) = send {
                // Only what the engine's rules allow reaches the interface.
                if let Some(cursor) = state.cursor {
                    assert!((1..=23).contains(&cursor.row) && cursor.column < 40);
                    // The browser echoes nothing: no flag but block is told.
                    assert_eq!(cursor.flags & !BLOCK, 0);
                }
            }
        }
    }
});
