//! A rendered page from a decoder: its palette, the colour of the screen
//! around it, and one palette index per dot, written straight from the
//! decoder's own buffers and checked whole on arrival.

use std::io::Write;

use super::kinds::KIND_FRAME;
use crate::frame::{MAX_COLOURS, PAGE_DOTS, RGB_BYTES};
use crate::{Fields, ProtocolError, write_frame};

/// A frame's page number and its count of colours, before the palette,
/// and its screen colour after it.
const FRAME_HEADER_BYTES: usize = 2 + 1 + 1;

/// Writes a page's frame, [`FromDecoder::Frame`], from the decoder's own
/// palette and indices: a frame is 240 KB, and this way it is not copied
/// into a message first.
///
/// # Errors
/// When writing fails or the frame is not a page's shape.
pub fn write_frame_to_core(out: &mut dyn Write, page: u16, palette: &[[u8; 3]], screen: u8, indices: &[u8]) -> Result<(), ProtocolError> {
    if palette.is_empty() || palette.len() > MAX_COLOURS || indices.len() != PAGE_DOTS || usize::from(screen) >= palette.len() {
        return Err(ProtocolError::BadValue("frame shape"));
    }
    let mut payload = Vec::with_capacity(FRAME_HEADER_BYTES.saturating_add(palette.len().saturating_mul(RGB_BYTES)).saturating_add(PAGE_DOTS));
    payload.extend_from_slice(&page.to_be_bytes());
    payload.push(u8::try_from(palette.len()).unwrap_or(0));
    for colour in palette {
        payload.extend_from_slice(colour);
    }
    payload.push(screen);
    payload.extend_from_slice(indices);
    write_frame(out, KIND_FRAME, &payload)
}

/// A frame's page, palette and screen colour, its indices checked against
/// the palette; the indices are the payload's tail, which the caller moves.
pub(super) fn read_frame(fields: &mut Fields<'_>) -> Result<(u16, Vec<[u8; 3]>, u8), ProtocolError> {
    let page = fields.display_page()?;
    let colours = usize::from(fields.u8()?);
    if colours == 0 || colours > MAX_COLOURS {
        return Err(ProtocolError::BadValue("palette size"));
    }
    let palette = fields.take(colours.saturating_mul(RGB_BYTES))?.as_chunks::<RGB_BYTES>().0.to_vec();
    let screen = fields.u8()?;
    if usize::from(screen) >= colours {
        return Err(ProtocolError::BadValue("screen colour outside the palette"));
    }
    let indices = fields.take(PAGE_DOTS)?;
    if indices.iter().any(|&index| usize::from(index) >= colours) {
        return Err(ProtocolError::BadValue("index outside the palette"));
    }
    fields.end()?;
    Ok((page, palette, screen))
}
