//! A rendered page from a decoder: its palette, the colour of the screen
//! around it, and one palette index per dot, written straight from the
//! decoder's own buffers and checked whole on arrival.

use std::io::Write;

use super::kinds::{KIND_FRAME, KIND_FRAME_AREA, KIND_REGIS};
use crate::frame::{MAX_COLOURS, PAGE_DOTS, RGB_BYTES};
use crate::{Fields, ProtocolError, write_frame};
use interface_core::canvas::{PAGE_HEIGHT, PAGE_WIDTH};

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
    let page = fields.channel_page()?;
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

/// A rectangle of a teletext page, in its dots (animation.md ANIM-5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PageArea {
    /// The left column.
    pub x: u16,
    /// The top row.
    pub y: u16,
    /// Columns.
    pub width: u16,
    /// Rows.
    pub height: u16,
}

impl PageArea {
    /// Its dots.
    #[must_use]
    pub fn dots(self) -> usize {
        usize::from(self.width).saturating_mul(usize::from(self.height))
    }

    /// Whether it lies in the page and holds a dot.
    #[must_use]
    pub fn fits(self) -> bool {
        self.dots() != 0
            && u32::from(self.x).saturating_add(u32::from(self.width)) <= u32::from(PAGE_WIDTH)
            && u32::from(self.y).saturating_add(u32::from(self.height)) <= u32::from(PAGE_HEIGHT)
    }
}

/// Checks a frame's palette, screen colour and indices.
fn check_frame(palette_len: usize, screen: u8, indices: &[u8]) -> Result<(), ProtocolError> {
    if palette_len == 0 || palette_len > MAX_COLOURS {
        return Err(ProtocolError::BadValue("palette size"));
    }
    if usize::from(screen) >= palette_len {
        return Err(ProtocolError::BadValue("screen colour outside the palette"));
    }
    if indices.iter().any(|&index| usize::from(index) >= palette_len) {
        return Err(ProtocolError::BadValue("index outside the palette"));
    }
    Ok(())
}

/// Writes the rectangle of a page that changed, [`FromDecoder::FrameArea`].
///
/// # Errors
/// When writing fails, or the area is outside the page or its dots are
/// not the area's.
pub(super) fn write_frame_area_to_core(
    out: &mut dyn Write,
    page: u16,
    palette: &[[u8; 3]],
    screen: u8,
    area: PageArea,
    indices: &[u8],
) -> Result<(), ProtocolError> {
    if !area.fits() || indices.len() != area.dots() {
        return Err(ProtocolError::BadValue("frame area"));
    }
    check_frame(palette.len(), screen, indices)?;
    let mut payload =
        Vec::with_capacity(FRAME_HEADER_BYTES.saturating_add(palette.len().saturating_mul(RGB_BYTES)).saturating_add(8).saturating_add(indices.len()));
    payload.extend_from_slice(&page.to_be_bytes());
    payload.push(u8::try_from(palette.len()).unwrap_or(0));
    for colour in palette {
        payload.extend_from_slice(colour);
    }
    payload.push(screen);
    for field in [area.x, area.y, area.width, area.height] {
        payload.extend_from_slice(&field.to_be_bytes());
    }
    payload.extend_from_slice(indices);
    write_frame(out, KIND_FRAME_AREA, &payload)
}

/// Reads what [`write_frame_area_to_core`] wrote, every field checked.
pub(super) fn read_frame_area(fields: &mut Fields<'_>) -> Result<super::FromDecoder, ProtocolError> {
    let page = fields.channel_page()?;
    let colours = usize::from(fields.u8()?);
    if colours == 0 || colours > MAX_COLOURS {
        return Err(ProtocolError::BadValue("palette size"));
    }
    let palette = fields.take(colours.saturating_mul(RGB_BYTES))?.as_chunks::<RGB_BYTES>().0.to_vec();
    let screen = fields.u8()?;
    let area = PageArea { x: fields.u16()?, y: fields.u16()?, width: fields.u16()?, height: fields.u16()? };
    if !area.fits() {
        return Err(ProtocolError::BadValue("frame area"));
    }
    let indices = fields.take(area.dots())?.to_vec();
    check_frame(colours, screen, &indices)?;
    fields.end()?;
    Ok(super::FromDecoder::FrameArea { page, palette, screen, area, indices })
}

/// Writes the picture of a sub-page drawn in ReGIS and its pick record,
/// [`FromDecoder::Regis`]: the page and sub-code, the pick record when it
/// is sent, then the picture or the part of it.
///
/// # Errors
/// When writing fails, the page is not a channel's, or the picture or its
/// pick record breaks a bound.
pub(super) fn write_regis_to_core(
    out: &mut dyn Write,
    page: u16,
    sub: u16,
    picture: &crate::regis::RegisPicture,
    pick: Option<&crate::regis::RegisPick>,
) -> Result<(), ProtocolError> {
    crate::fields::check_page(page)?;
    let mut payload = Vec::new();
    payload.extend_from_slice(&page.to_be_bytes());
    payload.extend_from_slice(&sub.to_be_bytes());
    // The picture comes first: its language says the size its pick record
    // covers (REQ-5819).
    crate::regis::push_picture(&mut payload, picture)?;
    payload.push(u8::from(pick.is_some()));
    if let Some(pick) = pick {
        crate::regis::push_pick(&mut payload, pick, picture.language)?;
    }
    write_frame(out, KIND_REGIS, &payload)
}
