//! The session file, which another program can write (requirement 20.23):
//! whatever it holds, reading it keeps every bound, and what was read
//! writes a file that reads back the same.
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext::restore::{MAX_ENTRIES, MAX_FILE, MAX_TABS, Saved};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let (saved, _) = Saved::read(&text);
    assert!(saved.tabs.len() <= MAX_TABS);
    assert!(saved.tabs.is_empty() || saved.active < saved.tabs.len());
    for tab in &saved.tabs {
        assert!(!tab.entries.is_empty() && tab.entries.len() <= MAX_ENTRIES && tab.position < tab.entries.len());
    }
    assert!(saved.window.is_none_or(|window| window.is_plausible()));
    let written = saved.text();
    assert!(written.len() <= MAX_FILE, "a file read within its bound writes back within it");
    let (again, dropped) = Saved::read(&written);
    assert_eq!(again, saved);
    assert!(dropped.is_empty(), "{dropped:?}");
});
