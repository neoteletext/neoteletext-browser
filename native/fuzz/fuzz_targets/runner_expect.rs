//! The standalone runner's `expect.json` reader (script-api.md section
//! 16; `script_run::expect`): arbitrary bytes are read or refused, never
//! a panic; every file named is one a run keeps, so that a kept set can
//! name no path outside its folder; and what is read is written back to
//! the same reading.
#![no_main]

use libfuzzer_sys::fuzz_target;
use script_run::expect::{parse, write};
use script_run::limits::MAX_EXPECT_FILES;
use script_run::run::is_kept;

fuzz_target!(|data: &[u8]| {
    let Ok(kept) = parse(data) else { return };
    assert!(kept.files.len() <= MAX_EXPECT_FILES);
    for (name, digest) in &kept.files {
        assert!(is_kept(name) && !name.contains('/') && !name.contains('\\'), "{name}");
        assert_eq!(digest.len(), 64);
    }
    assert_eq!(parse(&write(&kept)).as_ref(), Ok(&kept));
});
