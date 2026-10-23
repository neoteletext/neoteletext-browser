//! The launcher's state file, which anything running as the user can
//! write (`installation-and-updates.md` section 3): whatever it holds,
//! reading it keeps every bound, and what was read writes a file that
//! reads back the same.
#![no_main]

use libfuzzer_sys::fuzz_target;
use update::state::{MAX_BAD, State};

fuzz_target!(|data: &[u8]| {
    if let Ok(state) = State::parse(data) {
        assert!(state.bad.len() <= MAX_BAD);
        assert_eq!(State::parse(state.to_text().as_bytes()), Ok(state));
    }
});
