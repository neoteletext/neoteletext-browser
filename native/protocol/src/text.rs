//! Untrusted text on the pipes: cleaned before it is shown, checked clean
//! on arrival, and, from a helper process, carried raw for the trusted
//! core to clean.

/// Controls, and the invisible characters that reorder or hide text, are
/// never shown from untrusted text (requirement 11.5).
use interface_core::editor::hidden;

use crate::{Fields, ProtocolError};

/// Untrusted text made safe to show: hidden characters removed, then cut
/// to at most `max_bytes` on a character boundary.
#[must_use]
pub fn clean(text: &str, max_bytes: usize) -> String {
    let mut out = String::with_capacity(text.len().min(max_bytes));
    for character in text.chars().filter(|&character| !hidden(character)) {
        if out.len().saturating_add(character.len_utf8()) > max_bytes {
            break;
        }
        out.push(character);
    }
    out
}

/// Untrusted text sent on for the trusted core to clean: cut to at most
/// `limit` bytes on a character boundary, with a two-byte length before it.
pub(crate) fn push_raw_text(payload: &mut Vec<u8>, text: &str, limit: usize) {
    let end = (0..=text.len().min(limit)).rev().find(|&end| text.is_char_boundary(end)).unwrap_or(0);
    let cut = text.get(..end).unwrap_or_default();
    payload.extend_from_slice(&u16::try_from(cut.len()).unwrap_or(0).to_be_bytes());
    payload.extend_from_slice(cut.as_bytes());
}

pub(crate) fn is_clean(text: &str, max_bytes: usize) -> bool {
    text.len() <= max_bytes && !text.chars().any(hidden)
}

/// Text a helper process found, of at most `limit` bytes: invalid UTF-8 is
/// replaced, never trusted, and the trusted core cleans what remains; a
/// link's target, the trusted core's parser refuses if it is not a target.
pub(crate) fn raw_text(fields: &mut Fields<'_>, limit: usize, too_long: &'static str) -> Result<String, ProtocolError> {
    let length = usize::from(fields.u16()?);
    if length > limit {
        return Err(ProtocolError::BadValue(too_long));
    }
    Ok(String::from_utf8_lossy(fields.take(length)?).into_owned())
}
