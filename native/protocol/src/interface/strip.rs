//! The trusted strip's state on the wire: its flags, its texts, and the
//! code of a refused certificate (decision 83).

use interface_core::browser::{CertificateProblem, Denied, Failed, StripState};

use super::kinds::{ALL_FLAGS, BOOKMARKED, BUILT_IN, CAN_GO_BACK, CAN_GO_FORWARD, ENCRYPTED, LOCAL, PRIVATE};
use crate::fetcher::STATUS_CODES;
use crate::{Fields, MAX_ADDRESS, MAX_IDENTITY, MAX_STRIP_NOTICE, ProtocolError, push_text};

/// In place of a status: the channel refused nothing.
const NOT_DENIED: u16 = 0;
/// In place of a failure: the tab has not failed.
const NOT_FAILED: u8 = 0;
/// The tab's decoder failed.
const FAILED_DECODER: u8 = 1;
/// The tab's fetcher failed.
const FAILED_FETCHER: u8 = 2;
/// The address's channel file was refused.
const FAILED_CHANNEL: u8 = 3;
/// The page open was refused.
const FAILED_PAGE: u8 = 4;
/// In place of a status: nothing answered.
const NO_STATUS: u16 = 0;

pub(super) fn push_strip(payload: &mut Vec<u8>, strip: &StripState) -> Result<(), ProtocolError> {
    let flag = |set: bool, bit: u8| if set { bit } else { 0 };
    payload.push(
        flag(strip.encrypted, ENCRYPTED)
            | flag(strip.back, CAN_GO_BACK)
            | flag(strip.forward, CAN_GO_FORWARD)
            | flag(strip.bookmarked, BOOKMARKED)
            | flag(strip.local, LOCAL)
            | flag(strip.built_in, BUILT_IN)
            | flag(strip.private, PRIVATE),
    );
    push_text(payload, &strip.identity, MAX_IDENTITY)?;
    push_text(payload, &strip.address, MAX_ADDRESS)?;
    push_text(payload, &strip.notice, MAX_STRIP_NOTICE)?;
    payload.push(strip.refused.map_or(0, problem_code));
    push_text(payload, &strip.redirecting, MAX_ADDRESS)?;
    payload.extend_from_slice(&strip.denied.map_or(NOT_DENIED, Denied::status).to_be_bytes());
    let (failed, status) = match strip.failed {
        None => (NOT_FAILED, None),
        Some(Failed::Decoder) => (FAILED_DECODER, None),
        Some(Failed::Fetcher) => (FAILED_FETCHER, None),
        Some(Failed::Channel(status)) => (FAILED_CHANNEL, status),
        Some(Failed::Page(status)) => (FAILED_PAGE, status),
    };
    payload.push(failed);
    payload.extend_from_slice(&status.unwrap_or(NO_STATUS).to_be_bytes());
    Ok(())
}

pub(super) fn read_strip(fields: &mut Fields<'_>) -> Result<StripState, ProtocolError> {
    let flags = fields.u8()?;
    if flags & !ALL_FLAGS != 0 {
        return Err(ProtocolError::BadValue("strip flags"));
    }
    let identity = fields.text(MAX_IDENTITY)?;
    Ok(StripState {
        address: fields.text(MAX_ADDRESS)?,
        identity,
        encrypted: flags & ENCRYPTED != 0,
        back: flags & CAN_GO_BACK != 0,
        forward: flags & CAN_GO_FORWARD != 0,
        bookmarked: flags & BOOKMARKED != 0,
        local: flags & LOCAL != 0,
        built_in: flags & BUILT_IN != 0,
        private: flags & PRIVATE != 0,
        notice: fields.text(MAX_STRIP_NOTICE)?,
        refused: match fields.u8()? {
            0 => None,
            code => Some(problem_of(code).ok_or(ProtocolError::BadValue("certificate problem"))?),
        },
        redirecting: fields.text(MAX_ADDRESS)?,
        denied: match fields.u16()? {
            NOT_DENIED => None,
            status => Some(Denied::of_status(status).ok_or(ProtocolError::BadValue("denied status"))?),
        },
        failed: {
            let failed = fields.u8()?;
            let status = match fields.u16()? {
                NO_STATUS => None,
                status if STATUS_CODES.contains(&status) => Some(status),
                _ => return Err(ProtocolError::BadValue("failed tab's status")),
            };
            match failed {
                NOT_FAILED => None,
                FAILED_DECODER => Some(Failed::Decoder),
                FAILED_FETCHER => Some(Failed::Fetcher),
                FAILED_CHANNEL => Some(Failed::Channel(status)),
                FAILED_PAGE => Some(Failed::Page(status)),
                _ => return Err(ProtocolError::BadValue("failed tab")),
            }
        },
    })
}

/// A certificate problem's code on the wire: its place among
/// [`CertificateProblem::ALL`], from 1; 0 is none.
#[must_use]
pub fn problem_code(problem: CertificateProblem) -> u8 {
    let place = CertificateProblem::ALL.iter().position(|each| *each == problem).unwrap_or(0);
    u8::try_from(place).map_or(u8::MAX, |place| place.saturating_add(1))
}

/// The certificate problem of a code, when it names one.
#[must_use]
pub fn problem_of(code: u8) -> Option<CertificateProblem> {
    CertificateProblem::ALL.get(usize::from(code).checked_sub(1)?).copied()
}
