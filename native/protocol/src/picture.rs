//! The picture converter's messages (`docs/picture-tools.md`): the panel's
//! choices, from the interface to the trusted core and on to a decoder
//! started to convert one picture; the page that decoder made; and what the
//! trusted core tells the panel. A decoder's answer is untrusted: its pages
//! are bounded here and pass to a channel's decoder unread, and its words
//! are cleaned by the trusted core.

use interface_core::picture::{MAX_STATUS, PictureChoice, PictureStatus};

use crate::{Fields, ProtocolError, push_raw_text, push_text, raw_text};

/// The most bytes of a converted page in either form. A Level 3.5 page with
/// its rows in sixteen object sub-pages, the most the engine writes, is
/// about 30 KB as TTI and as T42; the bound leaves room twice over and
/// keeps both forms in one message under the framing's 256 KB.
pub const MAX_PICTURE_PAGE: usize = 96 * 1024;
const _: () = assert!(2 * MAX_PICTURE_PAGE + MAX_STATUS + 16 < crate::MAX_PAYLOAD, "one message holds both forms and the summary");

/// The largest picture file the broker reads and a decoder converts: the
/// engine converter's `MAX_INPUT_BYTES`, which the decoder, linking both,
/// asserts this equals (requirement 2.4).
pub const MAX_PICTURE_FILE: usize = 64 * 1024 * 1024;

/// The bytes of a choice: level, fit, alignment, background, hold mosaics.
const CHOICE_BYTES: usize = 5;

/// Writes a choice.
///
/// # Errors
/// When an index is outside its list.
pub(crate) fn push_choice(payload: &mut Vec<u8>, choice: PictureChoice) -> Result<(), ProtocolError> {
    if !choice.is_valid() {
        return Err(ProtocolError::BadValue("picture choice"));
    }
    payload.extend_from_slice(&[choice.level, choice.fit, choice.align, choice.background, u8::from(choice.hold_mosaics)]);
    Ok(())
}

/// Reads what [`push_choice`] wrote.
///
/// # Errors
/// When an index is outside its list, or the payload is short.
pub(crate) fn read_choice(fields: &mut Fields<'_>) -> Result<PictureChoice, ProtocolError> {
    let [level, fit, align, background, hold] = fields.array::<CHOICE_BYTES>()?;
    if hold > 1 {
        return Err(ProtocolError::BadValue("picture choice"));
    }
    let choice = PictureChoice { level, fit, align, background, hold_mosaics: hold == 1 };
    if choice.is_valid() { Ok(choice) } else { Err(ProtocolError::BadValue("picture choice")) }
}

/// Writes the panel's status: the trusted core's own, cleaned text.
///
/// # Errors
/// When the text is not clean or too long.
pub(crate) fn push_status(payload: &mut Vec<u8>, status: &PictureStatus) -> Result<(), ProtocolError> {
    payload.push(u8::from(status.converted));
    push_text(payload, &status.text, MAX_STATUS)
}

/// Reads what [`push_status`] wrote.
///
/// # Errors
/// When the payload is malformed.
pub(crate) fn read_status(fields: &mut Fields<'_>) -> Result<PictureStatus, ProtocolError> {
    let converted = fields.flag("converted")?;
    Ok(PictureStatus { converted, text: fields.text(MAX_STATUS)? })
}

/// Writes a page's bytes with a four-byte length.
///
/// # Errors
/// When it is longer than [`MAX_PICTURE_PAGE`].
fn push_page(payload: &mut Vec<u8>, page: &[u8]) -> Result<(), ProtocolError> {
    if page.len() > MAX_PICTURE_PAGE {
        return Err(ProtocolError::TooLong(page.len()));
    }
    payload.extend_from_slice(&u32::try_from(page.len()).map_err(|_| ProtocolError::TooLong(page.len()))?.to_be_bytes());
    payload.extend_from_slice(page);
    Ok(())
}

/// Reads what [`push_page`] wrote.
fn read_page(fields: &mut Fields<'_>) -> Result<Vec<u8>, ProtocolError> {
    let length = usize::try_from(fields.u32()?).map_err(|_| ProtocolError::BadValue("page length"))?;
    if length > MAX_PICTURE_PAGE {
        return Err(ProtocolError::TooLong(length));
    }
    Ok(fields.take(length)?.to_vec())
}

/// A converted page from a decoder: TTI, T42 and the converter's summary,
/// untrusted.
///
/// # Errors
/// When a form is too long.
pub(crate) fn push_converted(payload: &mut Vec<u8>, tti: &[u8], t42: &[u8], summary: &str) -> Result<(), ProtocolError> {
    push_page(payload, tti)?;
    push_page(payload, t42)?;
    push_raw_text(payload, summary, MAX_STATUS);
    Ok(())
}

/// Reads what [`push_converted`] wrote.
///
/// # Errors
/// When the payload is malformed or a form too long.
pub(crate) fn read_converted(fields: &mut Fields<'_>) -> Result<(Vec<u8>, Vec<u8>, String), ProtocolError> {
    let tti = read_page(fields)?;
    let t42 = read_page(fields)?;
    Ok((tti, t42, raw_text(fields, MAX_STATUS, "summary too long")?))
}
