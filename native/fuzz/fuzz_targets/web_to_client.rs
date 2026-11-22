//! What a web client reads (the script API's specification section 11.3,
//! DEC-7260 item 6, DEC-7268): a script host's requests, relayed by the
//! trusted core, and the trusted core's own messages. The same bytes are
//! read as a script host wrote them, relayed as the trusted core relays
//! them, and read as the web client reads them. What a script host may
//! not say is never relayed; a start for another partition is refused;
//! and every message read keeps to its bounds, so it is written again as
//! it was read.
#![no_main]

use libfuzzer_sys::fuzz_target;
use protocol::web::{PARTITION_TOKEN_BYTES, PartitionToken, ToWebClient, read_request, read_to_client, relay_to_client, write_request, write_to_client};

const TAB: u32 = 7;

fn own() -> PartitionToken {
    PartitionToken::new([0x5A; PARTITION_TOKEN_BYTES])
}

fn another() -> PartitionToken {
    PartitionToken::new([0x5B; PARTITION_TOKEN_BYTES])
}

fuzz_target!(|data: &[u8]| {
    // As a script host wrote them.
    let mut asked = Vec::new();
    let mut input = data;
    while let Ok(Some(message)) = read_request(&mut input) {
        if let ToWebClient::Start(start) = &message {
            assert!(start.check().is_ok());
        }
        assert!(!matches!(message, ToWebClient::TabCloses | ToWebClient::JarKept(_) | ToWebClient::ForgetJar), "the trusted core's alone");
        let mut written = Vec::new();
        write_request(&mut written, &message).expect("what was read is within its bounds");
        assert_eq!(read_request(&mut written.as_slice()).ok().flatten(), Some(message.clone()));
        asked.push(message);
    }
    // Relayed: whole messages only, never more than was written, and what
    // the web client then reads is what the script host said, for its tab.
    let (mut written, mut relayed) = (data.to_vec(), Vec::new());
    let _ = relay_to_client(TAB, &own(), &mut written, &mut relayed);
    assert!(written.len() <= data.len());
    let mut input = relayed.as_slice();
    let mut served = Vec::new();
    while let Ok(Some((tab, message))) = read_to_client(&mut input, &own()) {
        assert_eq!(tab, TAB);
        served.push(message);
    }
    assert_eq!(served, asked, "the web client is asked what the script host asked, and nothing else");
    // A web client of another partition serves no start of these.
    let mut input = relayed.as_slice();
    while let Ok(Some((_, message))) = read_to_client(&mut input, &another()) {
        assert!(!matches!(message, ToWebClient::Start(_)), "a start for another partition");
    }
    // As the web client reads whatever reaches its pipe.
    let mut input = data;
    while let Ok(Some((tab, message))) = read_to_client(&mut input, &own()) {
        if let ToWebClient::Start(start) = &message {
            assert!(start.check().is_ok());
        }
        let mut written = Vec::new();
        write_to_client(&mut written, tab, &own(), &message).expect("what was read is within its bounds");
        assert_eq!(read_to_client(&mut written.as_slice(), &own()).ok().flatten(), Some((tab, message)));
    }
});
