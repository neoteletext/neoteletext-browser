//! Link targets: the only page data the trusted core parses
//! (`hyperlinks.md` section 8.2, requirement 17.12).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = core::str::from_utf8(data) else { return };
    if let Some(neoteletext::links::Target::Web(address)) = neoteletext::links::parse_target(text) {
        // What is handed over is always an https or http address that the
        // opener's own check accepts.
        assert!(web_opener::acceptable(&address), "{address:?}");
    }
    core::hint::black_box(neoteletext::links::web_address(text));
});
