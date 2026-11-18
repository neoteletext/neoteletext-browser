//! Viewer state on the fetcher's pipe (neoteletext-common's
//! `docs/viewer-state.md` VS-119, DEC-6471): the fetcher tells the trusted
//! core what its channel's push connection said (a challenge, an end, an
//! offer to remember) and the core answers with what to send. Every message
//! is of a fixed length with enumerated values, so a fetcher compromised by
//! its server can name no channel, host or path: the core takes the channel
//! from the fetcher's own tab (VS-103) and builds the signed bytes itself.

use neoteletext_input::state::{Algorithm, CHALLENGE_BYTES, Lifetime, NoneReason, PUBLIC_KEY_BYTES, SIGNATURE_BYTES};

use super::kinds::{KIND_STATE_ANSWER, KIND_STATE_CHALLENGED, KIND_STATE_END_RECEIVED, KIND_STATE_KEEP_OFFERED, KIND_STATE_SEND_NONE, KIND_STATE_SEND_OFFER};
use super::{FromFetcher, ToFetcher};
use crate::{Fields, ProtocolError};

/// A key identifier's bytes: SHA-256 (VS-302), the engine's
/// `records::state::KEY_ID_BYTES`, which the fetcher asserts it equals.
pub const KEY_ID_BYTES: usize = 32;

/// The trusted core's answer to one challenge (VS-119).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateReply {
    /// A proof: the algorithm, the lifetime the key has, its public key and
    /// the signature over the bytes of VS-203.
    Proof {
        /// The algorithm.
        algorithm: Algorithm,
        /// Visit or remembered.
        lifetime: Lifetime,
        /// The public key.
        public_key: [u8; PUBLIC_KEY_BYTES],
        /// The signature.
        signature: [u8; SIGNATURE_BYTES],
    },
    /// No proof, and why (VS-204's `STATE_NONE`).
    None(NoneReason),
}

const REPLY_NONE: u8 = 0;
const REPLY_PROOF: u8 = 1;

fn none_reason(code: u8) -> Result<NoneReason, ProtocolError> {
    NoneReason::from_code(code).ok_or(ProtocolError::BadValue("state reason"))
}

/// A fetcher's state message, written.
pub(super) fn push_from_fetcher(payload: &mut Vec<u8>, message: &FromFetcher) -> Result<u8, ProtocolError> {
    Ok(match message {
        FromFetcher::StateChallenged { request, challenge, create } => {
            payload.extend_from_slice(&request.to_be_bytes());
            payload.extend_from_slice(challenge);
            payload.push(u8::from(*create));
            KIND_STATE_CHALLENGED
        }
        FromFetcher::StateEndReceived { key_id } => {
            payload.extend_from_slice(key_id);
            KIND_STATE_END_RECEIVED
        }
        FromFetcher::StateKeepOffered => KIND_STATE_KEEP_OFFERED,
        _ => return Err(ProtocolError::BadValue("not a state message")),
    })
}

/// A fetcher's state message of `kind`, read: of its exact length, every
/// value one the protocol names.
pub(super) fn read_from_fetcher(kind: u8, fields: &mut Fields<'_>) -> Result<FromFetcher, ProtocolError> {
    Ok(match kind {
        KIND_STATE_CHALLENGED => {
            let request = fields.u32()?;
            let challenge: [u8; CHALLENGE_BYTES] = fields.array()?;
            FromFetcher::StateChallenged { request, challenge, create: fields.flag("create")? }
        }
        KIND_STATE_END_RECEIVED => FromFetcher::StateEndReceived { key_id: fields.array()? },
        KIND_STATE_KEEP_OFFERED => FromFetcher::StateKeepOffered,
        other => return Err(ProtocolError::UnknownKind(other)),
    })
}

/// The trusted core's state message, written.
pub(super) fn push_to_fetcher(payload: &mut Vec<u8>, message: &ToFetcher) -> Result<u8, ProtocolError> {
    Ok(match message {
        ToFetcher::StateAnswer { request, reply } => {
            payload.extend_from_slice(&request.to_be_bytes());
            match reply {
                StateReply::Proof { algorithm, lifetime, public_key, signature } => {
                    payload.extend_from_slice(&[REPLY_PROOF, algorithm.code(), lifetime.code()]);
                    payload.extend_from_slice(public_key);
                    payload.extend_from_slice(signature);
                }
                StateReply::None(reason) => payload.extend_from_slice(&[REPLY_NONE, reason.code()]),
            }
            KIND_STATE_ANSWER
        }
        ToFetcher::StateSendOffer => KIND_STATE_SEND_OFFER,
        ToFetcher::StateSendNone(reason) => {
            payload.push(reason.code());
            KIND_STATE_SEND_NONE
        }
        _ => return Err(ProtocolError::BadValue("not a state message")),
    })
}

/// The trusted core's state message of `kind`, read.
pub(super) fn read_to_fetcher(kind: u8, fields: &mut Fields<'_>) -> Result<ToFetcher, ProtocolError> {
    Ok(match kind {
        KIND_STATE_ANSWER => {
            let request = fields.u32()?;
            let reply = match fields.u8()? {
                REPLY_PROOF => {
                    let algorithm = Algorithm::from_code(fields.u8()?).ok_or(ProtocolError::BadValue("state algorithm"))?;
                    let lifetime = Lifetime::from_code(fields.u8()?).ok_or(ProtocolError::BadValue("state lifetime"))?;
                    StateReply::Proof { algorithm, lifetime, public_key: fields.array()?, signature: fields.array()? }
                }
                REPLY_NONE => StateReply::None(none_reason(fields.u8()?)?),
                _ => return Err(ProtocolError::BadValue("state reply")),
            };
            ToFetcher::StateAnswer { request, reply }
        }
        KIND_STATE_SEND_OFFER => ToFetcher::StateSendOffer,
        KIND_STATE_SEND_NONE => ToFetcher::StateSendNone(none_reason(fields.u8()?)?),
        other => return Err(ProtocolError::UnknownKind(other)),
    })
}
