//! A staged program's bytes, searched by the launcher for the version
//! mark (`installation-and-updates.md` section 5): whatever they hold, the
//! search ends, and finds a version only from one whole mark.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(version) = update::mark::find(data) {
        let mark = update::version_mark!("V").replace('V', &version.to_string());
        assert!(data.windows(mark.len()).any(|window| window == mark.as_bytes()));
    }
});
