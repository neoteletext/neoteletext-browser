//! A channel's `channel.ttx` as the standalone runner reads it
//! (script-api.md section 16; `script_run::files::channel`, over the
//! engine's channel descriptor and its `scripts` member, section 12.1).
//! Whatever the bytes, nothing panics; the scripts named are within the
//! engine's bound on the pages a channel may name a script for, on pages
//! a channel may have; and a live channel names none.
#![no_main]

use decoder::scripts::limits::MAX_NAMED_PAGES;
use libfuzzer_sys::fuzz_target;
use neoteletext_core::limits::{FIRST_PAGE_ADDRESS, LAST_PAGE_ADDRESS};
use script_run::files::channel;

/// The channel the file is read as.
const CHANNEL_URL: &str = "https://example.test/chan/";
/// The first and last page numbers a channel may have.
const PAGES: core::ops::RangeInclusive<u16> = FIRST_PAGE_ADDRESS..=LAST_PAGE_ADDRESS;

fuzz_target!(|data: &[u8]| {
    let Ok(read) = channel(data, CHANNEL_URL) else { return };
    assert!(read.page_scripts.len() <= MAX_NAMED_PAGES, "{} pages name a script", read.page_scripts.len());
    assert!(PAGES.contains(&read.initial_page), "the first page is {:X}", read.initial_page);
    for (page, _, name) in &read.page_scripts {
        assert!(PAGES.contains(page) && !name.is_empty() && !name.contains(['/', '\\']), "{page:X} {name:?}");
    }
    if read.scripts_ignored {
        assert!(read.channel_script.is_none() && read.page_scripts.is_empty(), "a live channel runs a script");
    }
});
