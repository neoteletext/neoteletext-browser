//! The session file, which another program can write (requirement 20.23):
//! whatever it holds, reading it keeps every bound, and what was read
//! writes a file that reads back the same.
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext::restore::{MAX_ENTRIES, MAX_FILE, MAX_TABS, MAX_WINDOWS, Saved};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let (saved, _) = Saved::read(&text);
    assert!(saved.windows.len() <= MAX_WINDOWS && saved.tab_count() <= MAX_TABS);
    for window in &saved.windows {
        assert!(!window.tabs.is_empty() && window.active < window.tabs.len());
        assert!(window.geometry.is_none_or(|geometry| geometry.is_plausible()));
        for tab in &window.tabs {
            assert!(!tab.entries.is_empty() && tab.entries.len() <= MAX_ENTRIES && tab.position < tab.entries.len() && tab.window == 0);
        }
    }
    let written = saved.text();
    assert!(written.len() <= MAX_FILE, "a file read within its bound writes back within it");
    let (again, dropped) = Saved::read(&written);
    assert_eq!(again, saved);
    assert!(dropped.is_empty(), "{dropped:?}");
});
