//! Messages between the trusted core and a fetcher in snapshot mode
//! (neoteletext-common's `docs/tab-thumbnails.md` section 9, REQ-6884
//! item 2): a set of their own, so that a snapshot fetcher has no message
//! that could carry anything of the viewer. From the trusted core there is
//! only [`ToSnapshot::Close`]; to it, that the process is confined, the
//! answer's bytes in pieces, and how it ended, one byte of kind with no
//! text from outside. No message of `protocol::fetcher` is read or written
//! on this pipe, and none of these on a tab's fetcher's.

use std::io::{Read, Write};

use crate::fetcher::STATUS_CODES;
use crate::{Fields, MAX_PAYLOAD, ProtocolError, read_frame, write_frame};

// The snapshot pipe's kinds, in the next numbers free across every pipe
// on 19 November 2026, after the interface's 209.
const KIND_CLOSE: u8 = 210;
const KIND_CONFINED: u8 = 211;
const KIND_PIECE: u8 = 212;
const KIND_ENDED: u8 = 213;

/// The largest answer a snapshot keeps, in bytes: the largest sub-page
/// file with its picture, the fetcher's `pictured::MAX_PICTURED_SUBPAGE_FILE`
/// (the engine's DEC-6810), which the fetcher asserts it equals
/// (`docs/tab-thumbnails.md` section 11). The fetcher stops past it, the
/// trusted core counts the pieces against it, and the store keeps a body of
/// at most it.
pub const MAX_SNAPSHOT_BYTES: usize = 1_965_054;

/// How a snapshot's fetch ended: one byte of kind, and a status for a
/// refusal, never text from outside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotEnd {
    /// The server answered with a page (and closed with code 1000), or a
    /// static channel's file came: the pieces are the answer.
    Answered,
    /// The server refused with a DENIED record of this status.
    Denied(u16),
    /// The server did not agree SNAPSHOT: an older server, closed at once.
    NotOffered,
    /// The answer passed [`MAX_SNAPSHOT_BYTES`].
    TooLarge,
    /// Anything else, by kind.
    Failed(SnapshotFailure),
}

/// Why a snapshot's fetch failed, by kind alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotFailure {
    /// The connection could not be made or broke.
    Network,
    /// A time limit passed: the engine's `SNAPSHOT_TIME_LIMIT_US`.
    Timeout,
    /// The address was refused by the address rules, blocked as local,
    /// downgraded, or its TLS was unavailable.
    Address,
    /// An HTTP status other than the answer's, a missing file among them.
    Status,
    /// The answer broke a rule of HTTP, WebSocket or the push protocol.
    Protocol,
    /// Something else the engine reported.
    Other,
}

const END_ANSWERED: u8 = 0;
const END_DENIED: u8 = 1;
const END_NOT_OFFERED: u8 = 2;
const END_TOO_LARGE: u8 = 3;
const END_FAILED: u8 = 4;

const FAILED_NETWORK: u8 = 0;
const FAILED_TIMEOUT: u8 = 1;
const FAILED_ADDRESS: u8 = 2;
const FAILED_STATUS: u8 = 3;
const FAILED_PROTOCOL: u8 = 4;
const FAILED_OTHER: u8 = 5;

const fn failure_code(failure: SnapshotFailure) -> u8 {
    match failure {
        SnapshotFailure::Network => FAILED_NETWORK,
        SnapshotFailure::Timeout => FAILED_TIMEOUT,
        SnapshotFailure::Address => FAILED_ADDRESS,
        SnapshotFailure::Status => FAILED_STATUS,
        SnapshotFailure::Protocol => FAILED_PROTOCOL,
        SnapshotFailure::Other => FAILED_OTHER,
    }
}

const fn failure_of(code: u8) -> Result<SnapshotFailure, ProtocolError> {
    Ok(match code {
        FAILED_NETWORK => SnapshotFailure::Network,
        FAILED_TIMEOUT => SnapshotFailure::Timeout,
        FAILED_ADDRESS => SnapshotFailure::Address,
        FAILED_STATUS => SnapshotFailure::Status,
        FAILED_PROTOCOL => SnapshotFailure::Protocol,
        FAILED_OTHER => SnapshotFailure::Other,
        _ => return Err(ProtocolError::BadValue("snapshot failure")),
    })
}

/// From the trusted core to a snapshot fetcher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToSnapshot {
    /// Stop now: the snapshot is no longer wanted, or its time passed.
    Close,
}

/// From a snapshot fetcher to the trusted core. A fetcher is untrusted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FromSnapshot {
    /// It is confined and running, said first as every helper says it.
    Confined,
    /// The answer's bytes, in order, each within one message: whole
    /// packets as pushed, or a static channel's file as fetched.
    Piece(Vec<u8>),
    /// How it ended; it sends nothing after.
    Ended(SnapshotEnd),
}

/// # Errors
/// When writing fails.
pub fn write_to_fetcher(out: &mut dyn Write, message: ToSnapshot) -> Result<(), ProtocolError> {
    match message {
        ToSnapshot::Close => write_frame(out, KIND_CLOSE, &[]),
    }
}

/// Reads one message from the trusted core, or `None` at a clean end.
///
/// # Errors
/// On input errors, every kind but [`ToSnapshot::Close`]'s, and any
/// payload.
pub fn read_to_fetcher(input: &mut dyn Read) -> Result<Option<ToSnapshot>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    let message = match kind {
        KIND_CLOSE => ToSnapshot::Close,
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    Fields::new(&payload).end()?;
    Ok(Some(message))
}

/// # Errors
/// When writing fails, a piece is empty or longer than one message, or a
/// denial's status is not an HTTP status.
pub fn write_to_core(out: &mut dyn Write, message: &FromSnapshot) -> Result<(), ProtocolError> {
    match message {
        FromSnapshot::Confined => write_frame(out, KIND_CONFINED, &[]),
        FromSnapshot::Piece(bytes) => {
            check_piece(bytes)?;
            write_frame(out, KIND_PIECE, bytes)
        }
        FromSnapshot::Ended(end) => {
            let mut payload = Vec::with_capacity(3);
            match end {
                SnapshotEnd::Answered => payload.push(END_ANSWERED),
                SnapshotEnd::Denied(status) => {
                    check_status(*status)?;
                    payload.push(END_DENIED);
                    payload.extend_from_slice(&status.to_be_bytes());
                }
                SnapshotEnd::NotOffered => payload.push(END_NOT_OFFERED),
                SnapshotEnd::TooLarge => payload.push(END_TOO_LARGE),
                SnapshotEnd::Failed(failure) => payload.extend_from_slice(&[END_FAILED, failure_code(*failure)]),
            }
            write_frame(out, KIND_ENDED, &payload)
        }
    }
}

/// Reads one message from a snapshot fetcher, or `None` at a clean end.
///
/// # Errors
/// On input errors, a kind a snapshot fetcher may not send (every
/// `protocol::fetcher` kind among them), and any payload that is malformed.
pub fn read_from_fetcher(input: &mut dyn Read) -> Result<Option<FromSnapshot>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    let mut fields = Fields::new(&payload);
    let message = match kind {
        KIND_CONFINED => FromSnapshot::Confined,
        KIND_PIECE => {
            check_piece(&payload)?;
            return Ok(Some(FromSnapshot::Piece(payload)));
        }
        KIND_ENDED => FromSnapshot::Ended(match fields.u8()? {
            END_ANSWERED => SnapshotEnd::Answered,
            END_DENIED => {
                let status = fields.u16()?;
                check_status(status)?;
                SnapshotEnd::Denied(status)
            }
            END_NOT_OFFERED => SnapshotEnd::NotOffered,
            END_TOO_LARGE => SnapshotEnd::TooLarge,
            END_FAILED => SnapshotEnd::Failed(failure_of(fields.u8()?)?),
            _ => return Err(ProtocolError::BadValue("snapshot end")),
        }),
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    fields.end()?;
    Ok(Some(message))
}

fn check_piece(bytes: &[u8]) -> Result<(), ProtocolError> {
    if bytes.is_empty() {
        return Err(ProtocolError::BadValue("empty piece"));
    }
    if bytes.len() > MAX_PAYLOAD {
        return Err(ProtocolError::TooLong(bytes.len()));
    }
    Ok(())
}

fn check_status(status: u16) -> Result<(), ProtocolError> {
    if STATUS_CODES.contains(&status) { Ok(()) } else { Err(ProtocolError::BadValue("status")) }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]
mod tests {
    use super::*;

    fn every_message() -> Vec<FromSnapshot> {
        let mut messages = vec![
            FromSnapshot::Confined,
            FromSnapshot::Piece(vec![0x15; 42]),
            FromSnapshot::Piece(vec![1; MAX_PAYLOAD]),
            FromSnapshot::Ended(SnapshotEnd::Answered),
            FromSnapshot::Ended(SnapshotEnd::Denied(404)),
            FromSnapshot::Ended(SnapshotEnd::Denied(599)),
            FromSnapshot::Ended(SnapshotEnd::NotOffered),
            FromSnapshot::Ended(SnapshotEnd::TooLarge),
        ];
        for failure in [
            SnapshotFailure::Network,
            SnapshotFailure::Timeout,
            SnapshotFailure::Address,
            SnapshotFailure::Status,
            SnapshotFailure::Protocol,
            SnapshotFailure::Other,
        ] {
            messages.push(FromSnapshot::Ended(SnapshotEnd::Failed(failure)));
        }
        messages
    }

    /// REQ-6884 item 2: every message round trips, each side alone.
    #[test]
    fn every_snapshot_message_round_trips() {
        for message in every_message() {
            let mut out = Vec::new();
            write_to_core(&mut out, &message).unwrap();
            assert_eq!(read_from_fetcher(&mut out.as_slice()).unwrap(), Some(message));
        }
        let mut out = Vec::new();
        write_to_fetcher(&mut out, ToSnapshot::Close).unwrap();
        assert_eq!(read_to_fetcher(&mut out.as_slice()).unwrap(), Some(ToSnapshot::Close));
        assert_eq!(read_to_fetcher(&mut [].as_slice()).unwrap(), None);
    }

    /// REQ-6884 item 2: a snapshot fetcher accepts only `Close`, and the
    /// trusted core accepts from it only its three kinds: every other kind
    /// of every pipe, a tab fetcher's among them, is refused.
    #[test]
    fn every_other_kind_is_refused_on_both_sides() {
        for kind in 0..=u8::MAX {
            let frame = [kind, 0, 0, 0, 0];
            let to_fetcher = read_to_fetcher(&mut frame.as_slice());
            assert_eq!(to_fetcher.is_ok(), kind == KIND_CLOSE, "kind {kind} to a snapshot fetcher");
            if ![KIND_CONFINED, KIND_PIECE, KIND_ENDED].contains(&kind) {
                assert!(matches!(read_from_fetcher(&mut frame.as_slice()), Err(ProtocolError::UnknownKind(_))), "kind {kind} from one");
            }
        }
        // A tab fetcher's messages, written by its own writer, are refused.
        for message in [crate::fetcher::FromFetcher::Confined, crate::fetcher::FromFetcher::Notice("hello".into())] {
            let mut out = Vec::new();
            crate::fetcher::write_to_core(&mut out, &message).unwrap();
            assert!(read_from_fetcher(&mut out.as_slice()).is_err());
        }
        let mut out = Vec::new();
        crate::fetcher::write_to_fetcher(&mut out, &crate::fetcher::ToFetcher::FetchContents).unwrap();
        assert!(read_to_fetcher(&mut out.as_slice()).is_err());
    }

    /// Malformed payloads are refused: a close with a payload, an empty
    /// piece, an end of no kind, a denial's status that is no status, a
    /// failure of no kind, and an end that runs on.
    #[test]
    fn malformed_snapshot_messages_are_refused() {
        let frame = |kind: u8, payload: &[u8]| {
            let mut out = vec![kind];
            out.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
            out.extend_from_slice(payload);
            out
        };
        assert!(read_to_fetcher(&mut frame(KIND_CLOSE, &[0]).as_slice()).is_err());
        for payload in [&[][..], &[9], &[END_DENIED, 0, 99], &[END_DENIED, 2, 88], &[END_FAILED, 6], &[END_ANSWERED, 0], &[END_FAILED]] {
            assert!(read_from_fetcher(&mut frame(KIND_ENDED, payload).as_slice()).is_err(), "{payload:?}");
        }
        assert!(read_from_fetcher(&mut frame(KIND_PIECE, &[]).as_slice()).is_err());
        assert!(read_from_fetcher(&mut frame(KIND_CONFINED, &[1]).as_slice()).is_err());
        assert!(write_to_core(&mut Vec::new(), &FromSnapshot::Piece(Vec::new())).is_err());
        assert!(write_to_core(&mut Vec::new(), &FromSnapshot::Piece(vec![0; MAX_PAYLOAD + 1])).is_err());
        assert!(write_to_core(&mut Vec::new(), &FromSnapshot::Ended(SnapshotEnd::Denied(42))).is_err());
    }
}
