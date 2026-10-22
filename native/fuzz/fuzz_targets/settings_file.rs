//! The settings file, which another program can write (requirement 19.2):
//! whatever it holds, reading it keeps every bound, and what was read
//! writes a file that reads back the same.
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext::settings::{MAX_LOCAL, MAX_VOLUME, Settings};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let (settings, _) = Settings::read(&text);
    assert!(settings.volume <= MAX_VOLUME);
    assert!(settings.local.len() <= MAX_LOCAL);
    let (again, dropped) = Settings::read(&settings.text());
    assert_eq!(again, settings);
    assert!(dropped.is_empty(), "{dropped:?}");
});
