//! A release's manifest, downloaded by the updater and read by the
//! launcher (`installation-and-updates.md` section 6): whatever it holds,
//! reading it keeps every bound, names only the platform's own paths,
//! and what was read writes a manifest that reads back the same.
#![no_main]

use libfuzzer_sys::fuzz_target;
use update::manifest::{MAX_FILE_BYTES, Manifest};

fuzz_target!(|data: &[u8]| {
    if let Ok(manifest) = Manifest::parse(data) {
        for entry in &manifest.files {
            assert!(entry.length <= MAX_FILE_BYTES);
            assert!(manifest.platform.file(&entry.path).is_some(), "{}", entry.path);
        }
        assert_eq!(Manifest::parse(manifest.to_text().as_bytes()), Ok(manifest));
    }
});
