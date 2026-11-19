//! A decoder's snapshot mode on the wire (neoteletext-common's
//! `docs/tab-thumbnails.md` section 10, REQ-6885): the place asked for,
//! and the decoder's answer, the sub-page's source with concealed text
//! blanked in pieces, then its image or a refusal.

use crate::frame::{MAX_COLOURS, PAGE_DOTS, RGB_BYTES};
use crate::{Fields, MAX_PAYLOAD, ProtocolError};
use neoteletext_core::limits::is_sub_code;

use super::view::names_one_subpage;

/// A snapshot's image, as a still is made of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotImage {
    /// A teletext page, NAPLPS on the raster among them: palette indices,
    /// 480 by 500, as [`super::FromDecoder::Frame`] carries them.
    Frame {
        /// Red, green and blue of each colour the page uses.
        palette: Vec<[u8; 3]>,
        /// The palette index of the screen around the page.
        screen: u8,
        /// One palette index per dot, row by row.
        indices: Vec<u8>,
    },
    /// A ReGIS picture, whole, as [`super::FromDecoder::Regis`] carries it.
    Regis(crate::regis::RegisPicture),
}

/// Why a decoder in snapshot mode drew nothing (REQ-6885 item 3), one
/// byte: the trusted core says none of it, and treats each as a miss.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotRefusal {
    /// The bytes held no complete sub-page of the page asked for.
    NoSuchSubPage,
    /// A reader refused the bytes.
    Rejected,
    /// The picture faulted or passed its renderer's work allowance.
    PictureFailed,
    /// The source, with its picture, would pass `MAX_SNAPSHOT_BYTES`.
    TooLarge,
}

const IMAGE_FRAME: u8 = 0;
const IMAGE_REGIS: u8 = 1;

const REFUSED_NO_SUCH_SUBPAGE: u8 = 0;
const REFUSED_REJECTED: u8 = 1;
const REFUSED_PICTURE_FAILED: u8 = 2;
const REFUSED_TOO_LARGE: u8 = 3;

impl SnapshotRefusal {
    /// Its byte.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::NoSuchSubPage => REFUSED_NO_SUCH_SUBPAGE,
            Self::Rejected => REFUSED_REJECTED,
            Self::PictureFailed => REFUSED_PICTURE_FAILED,
            Self::TooLarge => REFUSED_TOO_LARGE,
        }
    }

    pub(super) const fn of(code: u8) -> Result<Self, ProtocolError> {
        Ok(match code {
            REFUSED_NO_SUCH_SUBPAGE => Self::NoSuchSubPage,
            REFUSED_REJECTED => Self::Rejected,
            REFUSED_PICTURE_FAILED => Self::PictureFailed,
            REFUSED_TOO_LARGE => Self::TooLarge,
            _ => return Err(ProtocolError::BadValue("snapshot refusal")),
        })
    }
}

/// The place a snapshot asks for: a display page, and one sub-page's code
/// or 3F7F for the page as a whole.
pub(super) fn check_asked(page: u16, sub: u16) -> Result<(), ProtocolError> {
    crate::fields::check_page(page)?;
    if is_sub_code(sub) { Ok(()) } else { Err(ProtocolError::BadValue("snapshot sub-code")) }
}

/// The place an answer names: the page, and the one sub-page drawn, or
/// for a refusal the sub-code asked.
fn check_answered(page: u16, sub: u16, drawn: bool) -> Result<(), ProtocolError> {
    crate::fields::check_page(page)?;
    let fits = if drawn { names_one_subpage(sub) } else { is_sub_code(sub) };
    if fits { Ok(()) } else { Err(ProtocolError::BadValue("snapshot sub-code")) }
}

pub(super) fn push_source(payload: &mut Vec<u8>, bytes: &[u8]) -> Result<(), ProtocolError> {
    if bytes.is_empty() || bytes.len() > MAX_PAYLOAD {
        return Err(ProtocolError::BadValue("snapshot source piece"));
    }
    payload.extend_from_slice(bytes);
    Ok(())
}

pub(super) fn push_drawn(payload: &mut Vec<u8>, page: u16, sub: u16, image: &SnapshotImage) -> Result<(), ProtocolError> {
    check_answered(page, sub, true)?;
    payload.extend_from_slice(&page.to_be_bytes());
    payload.extend_from_slice(&sub.to_be_bytes());
    match image {
        SnapshotImage::Frame { palette, screen, indices } => {
            if palette.is_empty() || palette.len() > MAX_COLOURS || usize::from(*screen) >= palette.len() || indices.len() != PAGE_DOTS {
                return Err(ProtocolError::BadValue("snapshot frame shape"));
            }
            if indices.iter().any(|&index| usize::from(index) >= palette.len()) {
                return Err(ProtocolError::BadValue("index outside the palette"));
            }
            payload.reserve(2_usize.saturating_add(palette.len().saturating_mul(RGB_BYTES)).saturating_add(PAGE_DOTS));
            payload.push(IMAGE_FRAME);
            payload.push(u8::try_from(palette.len()).map_err(|_| ProtocolError::BadValue("palette size"))?);
            for colour in palette {
                payload.extend_from_slice(colour);
            }
            payload.push(*screen);
            payload.extend_from_slice(indices);
        }
        SnapshotImage::Regis(picture) => {
            if !picture.whole() {
                return Err(ProtocolError::BadValue("a snapshot's picture is whole"));
            }
            payload.push(IMAGE_REGIS);
            crate::regis::push_picture(payload, picture)?;
        }
    }
    Ok(())
}

fn read_drawn(fields: &mut Fields<'_>) -> Result<(u16, u16, SnapshotImage), ProtocolError> {
    let page = fields.u16()?;
    let sub = fields.u16()?;
    check_answered(page, sub, true)?;
    let image = match fields.u8()? {
        IMAGE_FRAME => {
            let colours = usize::from(fields.u8()?);
            if colours == 0 || colours > MAX_COLOURS {
                return Err(ProtocolError::BadValue("palette size"));
            }
            let palette = fields.take(colours.saturating_mul(RGB_BYTES))?.as_chunks::<RGB_BYTES>().0.to_vec();
            let screen = fields.u8()?;
            if usize::from(screen) >= colours {
                return Err(ProtocolError::BadValue("screen colour outside the palette"));
            }
            let indices = fields.take(PAGE_DOTS)?.to_vec();
            if indices.iter().any(|&index| usize::from(index) >= colours) {
                return Err(ProtocolError::BadValue("index outside the palette"));
            }
            SnapshotImage::Frame { palette, screen, indices }
        }
        IMAGE_REGIS => {
            let picture = crate::regis::read_picture(fields)?;
            if !picture.whole() {
                return Err(ProtocolError::BadValue("a snapshot's picture is whole"));
            }
            SnapshotImage::Regis(picture)
        }
        _ => return Err(ProtocolError::BadValue("snapshot image")),
    };
    Ok((page, sub, image))
}

pub(super) fn push_refused(payload: &mut Vec<u8>, page: u16, sub: u16, reason: SnapshotRefusal) -> Result<(), ProtocolError> {
    check_answered(page, sub, false)?;
    payload.extend_from_slice(&page.to_be_bytes());
    payload.extend_from_slice(&sub.to_be_bytes());
    payload.push(reason.code());
    Ok(())
}

fn read_refused(fields: &mut Fields<'_>) -> Result<(u16, u16, SnapshotRefusal), ProtocolError> {
    let page = fields.u16()?;
    let sub = fields.u16()?;
    check_answered(page, sub, false)?;
    Ok((page, sub, SnapshotRefusal::of(fields.u8()?)?))
}

/// Reads a snapshot answer from a decoder, untrusted: a source piece is
/// the payload, moved.
pub(super) fn read_from_decoder(kind: u8, payload: Vec<u8>) -> Result<super::FromDecoder, ProtocolError> {
    if kind == super::kinds::KIND_SNAPSHOT_SOURCE {
        if payload.is_empty() {
            return Err(ProtocolError::BadValue("snapshot source piece"));
        }
        return Ok(super::FromDecoder::SnapshotSource(payload));
    }
    let mut fields = Fields::new(&payload);
    let message = if kind == super::kinds::KIND_SNAPSHOT_DRAWN {
        let (page, sub, image) = read_drawn(&mut fields)?;
        super::FromDecoder::SnapshotDrawn { page, sub, image }
    } else {
        let (page, sub, reason) = read_refused(&mut fields)?;
        super::FromDecoder::SnapshotRefused { page, sub, reason }
    };
    fields.end()?;
    Ok(message)
}
