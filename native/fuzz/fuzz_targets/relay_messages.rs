//! What the phone keyboard process reads from the relay and the phone:
//! JSON, and the plaintext of sealed frames.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = neoteletext_qrkeyboard::json::Value::parse(data);
    let _ = neoteletext_qrkeyboard::message::read(data);
    if let Ok(text) = core::str::from_utf8(data) {
        let _ = neoteletext_qrkeyboard::message::from_base64url(text);
    }
});
