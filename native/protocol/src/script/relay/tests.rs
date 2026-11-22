//! The shared pipes' tests (the specification's sections 11.1 and 11.2):
//! each reader gives the script link's and the web client's messages to
//! whom they are for, passes on unread exactly the bytes written, and
//! refuses every kind its pipe's writer may not send.

use super::*;
use crate::decoder::{write_to_core, write_to_decoder};
use crate::script::batch::Batch;
use crate::script::{ContextId, FailReason, LINK_VERSION, Step, write_to_decoder as write_link_to_decoder, write_to_host};
use crate::web::{ToWebClient, write_answer, write_request};

/// One of each message the script link carries to a script host.
fn to_host() -> Vec<ToScriptHost> {
    vec![
        ToScriptHost::Hello { link_version: LINK_VERSION },
        ToScriptHost::Source { context: ContextId::Page, offset: 0, bytes: b"1".to_vec() },
        ToScriptHost::Step(Box::new(Step { sequence: 7, tick: 9, ..Step::default() })),
        ToScriptHost::ContextClose(ContextId::Page),
        ToScriptHost::Shutdown,
    ]
}

/// One of each message the script link carries to a decoder.
fn to_decoder() -> Vec<FromScriptHost> {
    vec![
        FromScriptHost::Ready { link_version: LINK_VERSION, api_max: 1 },
        FromScriptHost::Batch(Box::new(Batch::new(7))),
        FromScriptHost::Failed { context: ContextId::Channel, reason: FailReason::Time, text: "late".into() },
    ]
}

/// Section 11.1: on a decoder's output the trusted core reads the
/// decoder's own messages whole and passes the script link's on unread,
/// byte for byte, in the order written.
#[test]
fn a_decoders_output_is_its_own_messages_and_the_links_unread() {
    let mut written = Vec::new();
    write_to_core(&mut written, &FromDecoder::Confined).unwrap();
    let mut frames = Vec::new();
    for message in to_host() {
        let mut frame = Vec::new();
        write_to_host(&mut frame, &message).unwrap();
        written.extend_from_slice(&frame);
        frames.push(frame);
    }
    write_to_core(&mut written, &FromDecoder::Confined).unwrap();
    let mut input = written.as_slice();
    assert_eq!(read_decoder_output(&mut input).unwrap(), Some(DecoderOutput::Core(FromDecoder::Confined)));
    let kinds = [LinkKind::Hello, LinkKind::Source, LinkKind::Step, LinkKind::ContextClose, LinkKind::Shutdown];
    for (frame, kind) in frames.iter().zip(kinds) {
        let Some(DecoderOutput::ToHost(relayed)) = read_decoder_output(&mut input).unwrap() else { panic!("not relayed") };
        assert_eq!(relayed.frame(), frame.as_slice());
        assert_eq!(relayed.link_kind(), Some(kind));
        assert_eq!(read_host_input(&mut relayed.frame()).unwrap().map(|input| matches!(input, HostInput::Link(_))), Some(true));
    }
    assert_eq!(read_decoder_output(&mut input).unwrap(), Some(DecoderOutput::Core(FromDecoder::Confined)));
    assert_eq!(read_decoder_output(&mut input).unwrap(), None);
}

/// Section 11.1: a decoder may not write what only a script host or a
/// web client writes; its pipe's reader refuses them, as before.
#[test]
fn a_decoder_cannot_write_a_script_hosts_or_a_web_clients_message() {
    for message in to_decoder() {
        let mut written = Vec::new();
        write_link_to_decoder(&mut written, &message).unwrap();
        assert!(matches!(read_decoder_output(&mut written.as_slice()), Err(ProtocolError::UnknownKind(_))), "{message:?}");
    }
    let mut written = Vec::new();
    write_request(&mut written, &ToWebClient::Abort(1)).unwrap();
    assert!(matches!(read_decoder_output(&mut written.as_slice()), Err(ProtocolError::UnknownKind(_))));
}

/// Section 11.1 and TODO-7292's check "a frame of an unknown kind is not
/// relayed": on a script host's output the link's messages for the
/// decoder and the requests of the web client are passed on unread, and
/// every other kind byte there is, the decoder's side of the link, the
/// web client's answers and the trusted core's own messages to a web
/// client among them, is refused.
#[test]
fn a_script_hosts_output_is_relayed_by_kind_and_every_other_kind_refused() {
    for message in to_decoder() {
        let mut written = Vec::new();
        write_link_to_decoder(&mut written, &message).unwrap();
        let Some(HostOutput::ToDecoder(relayed)) = read_host_output(&mut written.as_slice()).unwrap() else { panic!("{message:?}") };
        assert_eq!(relayed.frame(), written.as_slice());
        assert_eq!(read_decoder_input(&mut relayed.frame()).unwrap(), Some(DecoderInput::Link(message)));
    }
    let mut written = Vec::new();
    write_request(&mut written, &ToWebClient::Abort(3)).unwrap();
    let Some(HostOutput::Request(relayed)) = read_host_output(&mut written.as_slice()).unwrap() else { panic!("a request") };
    assert_eq!(relayed.frame(), written.as_slice());
    assert_eq!(relayed.link_kind(), None);
    let mut passed = 0_usize;
    for kind in 0..=u8::MAX {
        let frame = [kind, 0, 0, 0, 0];
        match read_host_output(&mut frame.as_slice()) {
            Ok(Some(HostOutput::ToDecoder(relayed))) => {
                assert!(is_to_decoder_kind(kind) && relayed.kind() == kind);
                passed += 1;
            }
            Ok(Some(HostOutput::Request(_))) => {
                assert!(is_request_kind(kind));
                passed += 1;
            }
            Err(ProtocolError::UnknownKind(refused)) => assert!(refused == kind && !is_to_decoder_kind(kind) && !is_request_kind(kind)),
            other => panic!("kind {kind}: {other:?}"),
        }
    }
    // READY, BATCH and FAILED; and the eight requests' kinds.
    assert_eq!(passed, 3 + 8);
    for kind in 0..=u8::MAX {
        assert!(!(is_to_host_kind(kind) && (is_to_decoder_kind(kind) || is_request_kind(kind) || is_answer_kind(kind))), "kind {kind} goes two ways");
    }
}

/// Section 11.2: a length past the framing's bound is refused before any
/// room is made for it, and a message cut short is an error, not an end.
#[test]
fn a_relayed_message_is_held_to_the_framings_bound() {
    let too_long = u32::try_from(MAX_PAYLOAD + 1).unwrap().to_be_bytes();
    let frame = [&[KIND_BATCH][..], &too_long].concat();
    assert!(matches!(read_host_output(&mut frame.as_slice()), Err(ProtocolError::TooLong(length)) if length == MAX_PAYLOAD + 1));
    let frame = [&[KIND_STEP][..], &too_long].concat();
    assert!(matches!(read_decoder_output(&mut frame.as_slice()), Err(ProtocolError::TooLong(_))));
    let cut = [KIND_BATCH, 0, 0, 0, 9, 1, 2];
    assert!(matches!(read_host_output(&mut cut.as_slice()), Err(ProtocolError::Io(_))));
    assert!(matches!(read_host_output(&mut [KIND_BATCH, 0].as_slice()), Err(ProtocolError::Io(_))));
    let mut widest = vec![KIND_BATCH];
    widest.extend_from_slice(&u32::try_from(MAX_PAYLOAD).unwrap().to_be_bytes());
    widest.resize(HEADER_BYTES + MAX_PAYLOAD, 0);
    assert!(matches!(read_host_output(&mut widest.as_slice()), Ok(Some(HostOutput::ToDecoder(relayed))) if relayed.frame().len() == widest.len()));
}

/// Section 11.1: a script host reads the link's messages and the web
/// client's answers on its one input, each whole, and refuses the rest.
#[test]
fn a_script_hosts_input_is_the_link_and_the_web_clients_answers() {
    let mut written = Vec::new();
    for message in to_host() {
        write_to_host(&mut written, &message).unwrap();
    }
    write_answer(&mut written, &FromWebClient::End(4)).unwrap();
    let mut input = written.as_slice();
    for message in to_host() {
        assert_eq!(read_host_input(&mut input).unwrap(), Some(HostInput::Link(message)));
    }
    assert_eq!(read_host_input(&mut input).unwrap(), Some(HostInput::Answer(FromWebClient::End(4))));
    assert_eq!(read_host_input(&mut input).unwrap(), None);
    let mut own = Vec::new();
    write_link_to_decoder(&mut own, &FromScriptHost::Ready { link_version: LINK_VERSION, api_max: 1 }).unwrap();
    assert!(matches!(read_host_input(&mut own.as_slice()), Err(ProtocolError::UnknownKind(_))));
    let mut core = Vec::new();
    write_to_decoder(&mut core, &ToDecoder::Close).unwrap();
    assert!(matches!(read_host_input(&mut core.as_slice()), Err(ProtocolError::UnknownKind(_))));
    assert!(matches!(read_host_input(&mut [KIND_STEP].as_slice()), Err(ProtocolError::Io(_))), "cut short after its kind");
}

/// Section 11.2: a decoder reads the trusted core's messages and its
/// script host's on its one input; a script host's message that breaks a
/// bound is the script's end and not the decoder's, and the pipe stands
/// at the next message.
#[test]
fn a_decoders_input_survives_a_script_hosts_malformed_message() {
    let mut written = Vec::new();
    write_to_decoder(&mut written, &ToDecoder::Close).unwrap();
    // A FAILED whose reason is no reason.
    written.extend_from_slice(&[KIND_FAILED, 0, 0, 0, 6, ContextId::Page.code(), 99, 0, 0, 0, 0]);
    write_link_to_decoder(&mut written, &FromScriptHost::Ready { link_version: LINK_VERSION, api_max: 1 }).unwrap();
    write_to_decoder(&mut written, &ToDecoder::Close).unwrap();
    let mut input = written.as_slice();
    assert_eq!(read_decoder_input(&mut input).unwrap(), Some(DecoderInput::Core(ToDecoder::Close)));
    assert!(matches!(read_decoder_input(&mut input), Err(DecoderInputError::Link(ProtocolError::BadValue(_)))));
    assert_eq!(read_decoder_input(&mut input).unwrap(), Some(DecoderInput::Link(FromScriptHost::Ready { link_version: LINK_VERSION, api_max: 1 })));
    assert_eq!(read_decoder_input(&mut input).unwrap(), Some(DecoderInput::Core(ToDecoder::Close)));
    assert_eq!(read_decoder_input(&mut input).unwrap(), None);
    // The decoder's side of the link is not a decoder's to be given.
    let mut step = Vec::new();
    write_to_host(&mut step, &ToScriptHost::Shutdown).unwrap();
    assert!(matches!(read_decoder_input(&mut step.as_slice()), Err(DecoderInputError::Pipe(ProtocolError::UnknownKind(_)))));
}

/// Section 10.6: `STEP_DEADLINE` is the specification's 250 ms.
#[test]
fn the_step_deadline_is_the_specifications() {
    assert_eq!(STEP_DEADLINE, Duration::from_millis(250));
    assert_eq!(MAX_STEPS_UNANSWERED, 60);
}
