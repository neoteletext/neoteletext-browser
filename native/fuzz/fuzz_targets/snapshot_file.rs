//! A snapshot file of the store (REQ-6886, neoteletext-common's
//! `docs/tab-thumbnails.md` section 11), which another program can write
//! in the settings folder: whatever it holds, reading it for a fixed place
//! never panics, keeps the body within its bound, and a file accepted
//! writes back to exactly the bytes read.
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext::snapshots::{MAX_SNAPSHOT_FILE, Place, WHOLE_PAGE, decode, encode};

fuzz_target!(|data: &[u8]| {
    let place = Place { channel: String::from("https://a.example/tt/"), page: 0x100, sub: WHOLE_PAGE };
    let Ok(snapshot) = decode(&place, data) else { return };
    assert!(data.len() <= MAX_SNAPSHOT_FILE);
    assert_eq!(encode(&place, &snapshot).as_deref(), Ok(data), "a file accepted writes back the same");
});
