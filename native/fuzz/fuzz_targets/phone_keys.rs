//! What the trusted core reads from the phone keyboard process, which
//! talks to a relay and is untrusted (REQ-INT-138).
#![no_main]

use libfuzzer_sys::fuzz_target;
use protocol::keyboard::{FromKeyboard, MAX_KEY_NAME, MAX_PHONE_TEXT};

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    while let Ok(Some(message)) = protocol::keyboard::read_from_keyboard(&mut input) {
        match message {
            FromKeyboard::Key { name, .. } => assert!(name.len() <= MAX_KEY_NAME && !name.chars().any(char::is_control)),
            FromKeyboard::Text(text) => assert!(text.len() <= MAX_PHONE_TEXT),
            FromKeyboard::Join { qr, .. } => assert!(qr.size > 0 && qr.size <= protocol::keyboard::MAX_QR_SIZE),
            FromKeyboard::State(_) => {}
        }
    }
});
