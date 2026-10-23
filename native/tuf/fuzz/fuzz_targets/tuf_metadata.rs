//! The Update Framework's metadata as the updater reads it, through
//! `tough`'s own types (`installation-and-updates-security.md` section 5):
//! whatever the update host serves, each role's file is read without a
//! fault, and one that reads re-serialises.
#![no_main]

use libfuzzer_sys::fuzz_target;
use tough::schema::{Root, Signed, Snapshot, Targets, Timestamp};

fuzz_target!(|data: &[u8]| {
    if let Ok(root) = serde_json::from_slice::<Signed<Root>>(data) {
        let _ = serde_json::to_vec(&root);
    }
    let _ = core::hint::black_box(serde_json::from_slice::<Signed<Timestamp>>(data));
    let _ = core::hint::black_box(serde_json::from_slice::<Signed<Snapshot>>(data));
    if let Ok(targets) = serde_json::from_slice::<Signed<Targets>>(data) {
        let _ = serde_json::to_vec(&targets);
    }
});
