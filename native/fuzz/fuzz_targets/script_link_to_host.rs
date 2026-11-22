//! What a script host reads from its decoder (the script API's
//! specification section 11.2, DEC-7266): an opening, a script's source
//! in pieces, steps. Every message read keeps to its bounds, so it is
//! written again as it was read, and the decoder's reader takes none of
//! them.
#![no_main]

use libfuzzer_sys::fuzz_target;
use protocol::script::{
    MAX_EVENT_PAYLOAD_BYTES, MAX_SCRIPT_BYTES, MAX_SOURCE_PIECE_BYTES, MAX_STEP_EVENTS, ToScriptHost, is_to_decoder_kind, is_to_host_kind, read_to_decoder,
    read_to_host, write_to_host,
};

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    while let Ok(Some(message)) = read_to_host(&mut input) {
        match &message {
            ToScriptHost::ContextOpen(open) => assert!(open.check().is_ok()),
            ToScriptHost::Step(step) => {
                assert!(step.check().is_ok());
                assert!(step.events.len() <= MAX_STEP_EVENTS && step.events.iter().all(|event| event.payload.len() <= MAX_EVENT_PAYLOAD_BYTES));
            }
            ToScriptHost::Source { offset, bytes, .. } => {
                assert!(!bytes.is_empty() && bytes.len() <= MAX_SOURCE_PIECE_BYTES);
                assert!(usize::try_from(*offset).is_ok_and(|offset| offset + bytes.len() <= MAX_SCRIPT_BYTES));
            }
            ToScriptHost::Hello { .. } | ToScriptHost::ContextClose(_) | ToScriptHost::Shutdown => {}
        }
        let mut written = Vec::new();
        write_to_host(&mut written, &message).expect("what was read is within its bounds");
        assert!(written.first().is_some_and(|&kind| is_to_host_kind(kind) && !is_to_decoder_kind(kind)));
        assert_eq!(read_to_host(&mut written.as_slice()).ok().flatten(), Some(message));
        assert!(read_to_decoder(&mut written.as_slice()).is_err(), "a decoder's message is not a script host's");
    }
});
