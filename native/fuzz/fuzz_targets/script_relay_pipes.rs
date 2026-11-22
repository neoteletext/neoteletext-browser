//! The pipes the script link shares with other messages (the script
//! API's specification section 11.1, TODO-7292): what the trusted core
//! reads of a decoder's output and of a script host's, and what a script
//! host and a decoder read of their input. Whatever the bytes, no reader
//! panics or makes room past the framing's bound; a message the trusted
//! core passes on unread is exactly the bytes that were written and of a
//! kind its writer may send; and a kind one side may write is never one
//! the other may.
#![no_main]

use libfuzzer_sys::fuzz_target;
use protocol::MAX_PAYLOAD;
use protocol::script::relay::{DecoderInputError, DecoderOutput, HostOutput, read_decoder_input, read_decoder_output, read_host_input, read_host_output};
use protocol::script::{is_to_decoder_kind, is_to_host_kind};
use protocol::web::{is_answer_kind, is_request_kind};

/// The bytes before a payload: its kind and its length.
const HEADER_BYTES: usize = 5;

fuzz_target!(|data: &[u8]| {
    // A script host's output, as the trusted core reads it.
    let mut input = data;
    loop {
        let before = input;
        match read_host_output(&mut input) {
            Ok(Some(HostOutput::ToDecoder(relayed))) => {
                assert!(is_to_decoder_kind(relayed.kind()) && !is_to_host_kind(relayed.kind()));
                assert!(relayed.frame().len() <= HEADER_BYTES + MAX_PAYLOAD);
                assert_eq!(before.get(..relayed.frame().len()), Some(relayed.frame()), "passed on as it was written");
            }
            Ok(Some(HostOutput::Request(relayed))) => {
                assert!(is_request_kind(relayed.kind()) && !is_answer_kind(relayed.kind()) && relayed.link_kind().is_none());
                assert_eq!(before.get(..relayed.frame().len()), Some(relayed.frame()), "passed on as it was written");
            }
            Ok(None) | Err(_) => break,
        }
    }
    // A decoder's output, as the trusted core reads it.
    let mut input = data;
    loop {
        let before = input;
        match read_decoder_output(&mut input) {
            Ok(Some(DecoderOutput::ToHost(relayed))) => {
                assert!(is_to_host_kind(relayed.kind()) && relayed.link_kind().is_some());
                assert_eq!(before.get(..relayed.frame().len()), Some(relayed.frame()), "passed on as it was written");
                // What a script host makes of it is its own affair; it
                // must not panic.
                let _ = read_host_input(&mut relayed.frame());
            }
            Ok(Some(DecoderOutput::Core(_))) => {}
            Ok(None) | Err(_) => break,
        }
    }
    // A script host's input.
    let mut input = data;
    while let Ok(Some(_)) = read_host_input(&mut input) {}
    // A decoder's input: a refused message of the script link leaves the
    // pipe at the next message, so reading goes on.
    let mut input = data;
    while let Ok(Some(_)) | Err(DecoderInputError::Link(_)) = read_decoder_input(&mut input) {}
});
