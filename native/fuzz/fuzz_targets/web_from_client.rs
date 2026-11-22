//! What the trusted core and a script host read from a web client, which
//! talks to the network and is untrusted (the script API's specification
//! section 11.3, DEC-7260 item 6, DEC-7268). The same bytes are read
//! whole, relayed as the trusted core relays them, and read as a script
//! host reads them. A script host is never given the web client's
//! `Confined` or its jar, the relay reads no answer's payload and yet
//! passes nothing a script host's reader would take for another message,
//! and every message read keeps to its bounds.
#![no_main]

use libfuzzer_sys::fuzz_target;
use protocol::web::{FromClientFrame, FromWebClient, read_answer, read_from_client, read_request, relay_from_client, write_answer, write_from_client};

fuzz_target!(|data: &[u8]| {
    // Read whole, and written again as read.
    let mut input = data;
    let mut whole = Vec::new();
    while let Ok(Some((tab, message))) = read_from_client(&mut input) {
        if let FromWebClient::Head(head) = &message {
            assert!(head.check().is_ok());
        }
        let mut written = Vec::new();
        write_from_client(&mut written, tab, &message).expect("what was read is within its bounds");
        assert_eq!(read_from_client(&mut written.as_slice()).ok().flatten(), Some((tab, message.clone())));
        whole.push((tab, message));
    }
    // Relayed. Each frame the relay passes on is, to the script host's
    // reader, exactly the message the whole reader found there, or one it
    // refuses.
    let mut input = data;
    let mut expected = whole.into_iter();
    while let Ok(Some(frame)) = relay_from_client(&mut input) {
        match frame {
            FromClientFrame::Core(message) => {
                assert!(matches!(message, FromWebClient::Confined | FromWebClient::Jar(_)));
                assert!(write_answer(&mut Vec::new(), &message).is_err(), "never a script host's");
                assert_eq!(expected.next().map(|(_, message)| message), Some(message));
            }
            FromClientFrame::Answer { tab, frame } => {
                let mut bytes = frame.as_slice();
                // A payload that is no message: the whole reader stopped
                // here too, and nothing more is compared.
                let Ok(Some(message)) = read_answer(&mut bytes) else {
                    assert_eq!(expected.next(), None);
                    break;
                };
                assert!(bytes.is_empty(), "one frame, one message");
                assert!(!matches!(message, FromWebClient::Confined | FromWebClient::Jar(_)));
                assert_eq!(expected.next(), Some((tab, message)));
                assert!(read_request(&mut frame.as_slice()).is_err(), "an answer is not a request");
            }
        }
    }
    // As a script host reads whatever reaches its pipe.
    let mut input = data;
    while let Ok(Some(message)) = read_answer(&mut input) {
        assert!(!matches!(message, FromWebClient::Confined | FromWebClient::Jar(_)));
        let mut written = Vec::new();
        write_answer(&mut written, &message).expect("what was read is within its bounds");
        assert_eq!(read_answer(&mut written.as_slice()).ok().flatten(), Some(message));
    }
});
