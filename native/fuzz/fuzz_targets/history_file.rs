//! The history file, which another program can write (requirement 20.3):
//! whatever it holds, reading it keeps every bound, and what was read
//! writes a file that reads back the same.
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext::history::{History, MAX_FILE, MAX_ROWS};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let (history, _) = History::read(&text);
    assert!(history.len() <= MAX_ROWS);
    let written = history.text();
    assert!(written.len() <= MAX_FILE, "a file read within its bound writes back within it");
    let (again, dropped) = History::read(&written);
    assert_eq!(again.query(""), history.query(""));
    assert!(dropped.is_empty(), "{dropped:?}");
});
