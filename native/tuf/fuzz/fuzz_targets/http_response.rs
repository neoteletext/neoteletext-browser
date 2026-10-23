//! The update host's HTTP response as the updater reads it, by the
//! engine's rules (`installation-and-updates.md` section 8.2): whatever
//! arrives, the reader ends, and a body it returns is within its bound.
#![no_main]

use libfuzzer_sys::fuzz_target;
use updater::transport::{MAX_BODY, read_response};

fuzz_target!(|data: &[u8]| {
    if let Ok(Some(body)) = read_response(&mut &data[..]) {
        assert!(body.len() <= MAX_BODY);
    }
});
