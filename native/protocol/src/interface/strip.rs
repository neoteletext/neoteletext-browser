//! The trusted strip's state on the wire: its flags, its texts, and the
//! code of a refused certificate (decision 83).

use interface_core::browser::{CertificateProblem, StripState};

use super::kinds::{ALL_FLAGS, BOOKMARKED, BUILT_IN, CAN_GO_BACK, CAN_GO_FORWARD, ENCRYPTED, LOCAL, PRIVATE};
use crate::{Fields, MAX_ADDRESS, MAX_IDENTITY, MAX_STRIP_NOTICE, ProtocolError, push_text};

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
