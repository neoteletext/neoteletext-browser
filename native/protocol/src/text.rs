//! Untrusted text on the pipes: cleaned before it is shown, checked clean
//! on arrival, and, from a helper process, carried raw for the trusted
//! core to clean.

/// Controls, and the invisible characters that reorder or hide text, are
/// never shown from untrusted text (requirement 11.5).
use interface_core::editor::hidden;

use crate::{Fields, ProtocolError};

/// The most combining marks kept in a row after one character: polytonic
/// Greek, the most any orthography written with these blocks' marks needs,
/// stacks three on a letter (breathing, accent and iota subscript), and
/// one more is allowed.
pub const MAX_COMBINING_RUN: usize = 4;

/// Untrusted text made safe to show: hidden characters removed, a run of
/// combining marks cut at [`MAX_COMBINING_RUN`] (finding F-36), then cut to
/// at most `max_bytes` on a character boundary.
#[must_use]
pub fn clean(text: &str, max_bytes: usize) -> String {
    let mut out = String::with_capacity(text.len().min(max_bytes));
    let mut run = 0_usize;
    for character in text.chars().filter(|&character| !hidden(character)) {
        if combining(character) {
            run = run.saturating_add(1);
            if run > MAX_COMBINING_RUN {
                continue;
            }
        } else {
            run = 0;
        }
        if out.len().saturating_add(character.len_utf8()) > max_bytes {
            break;
        }
        out.push(character);
    }
    out
}

/// Whether a character is a combining mark of the blocks that hold only
/// combining marks for any script (Unicode 16.0): Combining Diacritical
/// Marks, its Extended and Supplement blocks, the marks for symbols, which
/// include the enclosing marks, and the half marks; and Cyrillic's
/// combining and enclosing marks. These are what text piling marks over
/// a line uses. A script's own marks (Devanagari's, Hebrew's) are not
/// counted: telling every mark needs Unicode's tables, which no approved
/// crate of this one carries.
fn combining(character: char) -> bool {
    matches!(character,
        '\u{0300}'..='\u{036F}'
        | '\u{0483}'..='\u{0489}'
        | '\u{1AB0}'..='\u{1AFF}'
        | '\u{1DC0}'..='\u{1DFF}'
        | '\u{20D0}'..='\u{20FF}'
        | '\u{FE20}'..='\u{FE2F}')
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Finding F-36: a run of combining marks is cut at
    /// `MAX_COMBINING_RUN`, so untrusted text cannot pile marks up and
    /// down the strip; the marks a language stacks are kept.
    #[test]
    fn a_run_of_combining_marks_is_cut() {
        let piled = format!("a{}b", "\u{0301}\u{0316}\u{20DD}\u{1DC0}\u{FE20}".repeat(20));
        let cleaned = clean(&piled, 1024);
        let expected = format!("a{}b", "\u{0301}\u{0316}\u{20DD}\u{1DC0}\u{FE20}".chars().cycle().take(MAX_COMBINING_RUN).collect::<String>());
        assert_eq!(cleaned, expected);
        // Polytonic Greek: alpha with psili, oxia and ypogegrammeni.
        assert_eq!(clean("\u{03B1}\u{0313}\u{0301}\u{0345}", 64), "\u{03B1}\u{0313}\u{0301}\u{0345}");
        // A hidden character between marks does not start a new run.
        let split = format!("a{}\u{200B}{}", "\u{0301}".repeat(MAX_COMBINING_RUN), "\u{0301}".repeat(MAX_COMBINING_RUN));
        assert_eq!(clean(&split, 1024), format!("a{}", "\u{0301}".repeat(MAX_COMBINING_RUN)));
    }
}
