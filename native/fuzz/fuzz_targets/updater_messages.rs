//! What the trusted core reads from the updater, the process that reads
//! the network, and what the updater reads from the core
//! (`installation-and-updates.md` section 3): a kind byte, then the
//! payload; whatever arrives is read within its bounds, and what was read
//! encodes to the same message.
#![no_main]

use libfuzzer_sys::fuzz_target;
use update::wire::{MAX_REASON, decode_from_updater, decode_to_updater, encode_from_updater, read_from_updater};

fuzz_target!(|data: &[u8]| {
    let Some((&kind, payload)) = data.split_first() else { return };
    if let Ok(message) = decode_from_updater(kind, payload) {
        if let update::wire::FromUpdater::Failed { reason, .. } = &message {
            assert!(reason.len() <= MAX_REASON);
        }
        let bytes = encode_from_updater(&message).unwrap();
        assert_eq!(read_from_updater(&mut bytes.as_slice()).unwrap(), message);
    }
    let _ = core::hint::black_box(decode_to_updater(kind, payload));
});
