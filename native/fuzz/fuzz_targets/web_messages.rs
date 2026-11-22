//! What the fetcher's general web client reads from the trusted core, and
//! the trusted core from it (neoteletext-common's DEC-7260 item 6): every
//! message read keeps to its bounds, so it is written again as it was
//! read, and neither side reads the other's kinds.
#![no_main]

use libfuzzer_sys::fuzz_target;
use protocol::web::{FromWebClient, ToWebClient, read_from_client, read_to_client, write_to_client, write_to_core};

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    while let Ok(Some(message)) = read_to_client(&mut input) {
        if let ToWebClient::Start(start) = &message {
            assert!(start.check().is_ok());
        }
        let mut written = Vec::new();
        write_to_client(&mut written, &message).expect("what was read is within its bounds");
        assert_eq!(read_to_client(&mut written.as_slice()).ok().flatten(), Some(message));
        assert!(read_from_client(&mut written.as_slice()).is_err(), "the trusted core's message is not a web client's");
    }
    let mut input = data;
    while let Ok(Some(message)) = read_from_client(&mut input) {
        if let FromWebClient::Head(head) = &message {
            assert!(head.check().is_ok());
        }
        let mut written = Vec::new();
        write_to_core(&mut written, &message).expect("what was read is within its bounds");
        assert_eq!(read_from_client(&mut written.as_slice()).ok().flatten(), Some(message));
        assert!(read_to_client(&mut written.as_slice()).is_err(), "a web client's message is not the trusted core's");
    }
});
