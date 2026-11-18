//! A picture drawn by the ReGIS renderer (DEC-6335) or the NAPLPS renderer
//! (DEC-6407), as the decoder sends it to the trusted core and the trusted
//! core to the interface: sixteen colours, packed two pixels to a byte so
//! that it fits one message, and with it for the trusted core alone the
//! pick record, which says which numbered drawing lies under each pixel. A
//! ReGIS picture is 800 by 480 pixels; a NAPLPS picture is the teletext
//! raster, 480 by 500 (the NAPLPS renderer's REQ-5819, DEC-6667). Its
//! language says which, and every size is checked against it.
//!
//! Everything here comes from a decoder, so every count and every index is
//! checked on reading.

use crate::{Fields, MAX_PAYLOAD, ProtocolError, push_flag, push_key};

/// A ReGIS picture's width in pixels.
pub const REGIS_WIDTH: usize = interface_core::canvas::REGIS_WIDTH as usize;
/// A ReGIS picture's height in pixels.
pub const REGIS_HEIGHT: usize = interface_core::canvas::REGIS_HEIGHT as usize;
/// The pixels of a ReGIS picture, the largest of any language.
pub const REGIS_PIXELS: usize = REGIS_WIDTH * REGIS_HEIGHT;
const _: () = assert!(PictureLanguage::Regis.pixels() == REGIS_PIXELS && PictureLanguage::Naplps.pixels() <= REGIS_PIXELS);
/// The colours of a picture: each pixel is one of sixteen.
pub const REGIS_COLOURS: usize = interface_core::canvas::REGIS_COLOURS;
/// A ReGIS picture's pixels packed two to a byte, the left one in the high
/// half: the most a picture of any language packs to.
pub const REGIS_PACKED: usize = REGIS_PIXELS / 2;
/// The most detectable drawings whose origins go with a pick record. A
/// picture with more has no pick record sent.
pub const MAX_PICK_ORIGINS: usize = 1024;
/// The most runs a pick record is sent in. A picture whose record needs
/// more has none sent.
pub const MAX_PICK_RUNS: usize = 12_000;
/// The longest run of a pick record.
const MAX_RUN: usize = u16::MAX as usize;
/// A picture's fields before its pixels: the commit count, the work units,
/// its language, the sixteen colours, its number and its base's, and its
/// area.
const PICTURE_FIXED: usize = 4 + 8 + 1 + REGIS_COLOURS * 3 + 4 + 4 + AREA_BYTES;
/// An area's bytes: its corner and its size.
const AREA_BYTES: usize = 2 + 2 + 2 + 2;
/// An origin's bytes: the drawing and its two coordinates.
const ORIGIN_BYTES: usize = 2 + 4 + 4;
/// A run's bytes: the drawing and its length.
const RUN_BYTES: usize = 2 + 2;
// The largest picture, the largest pick record and the page it is of fit
// one message.
const _: () = assert!(16 + PICTURE_FIXED + REGIS_PACKED + 2 + MAX_PICK_ORIGINS * ORIGIN_BYTES + 2 + MAX_PICK_RUNS * RUN_BYTES <= MAX_PAYLOAD);

/// A rectangle of a picture, in the picture's own pixels. Pixels travel
/// two to a byte, so its left edge and its width are even.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegisArea {
    /// The left column.
    pub x: u16,
    /// The top row.
    pub y: u16,
    /// Columns.
    pub width: u16,
    /// Rows.
    pub height: u16,
}

impl RegisArea {
    /// The whole of a picture drawn in `language`.
    #[must_use]
    pub const fn whole(language: PictureLanguage) -> Self {
        Self { x: 0, y: 0, width: language.width(), height: language.height() }
    }

    /// Its pixels.
    #[must_use]
    pub fn pixels(self) -> usize {
        usize::from(self.width).saturating_mul(usize::from(self.height))
    }

    /// Whether it lies in a picture drawn in `language` with an even left
    /// edge and width.
    #[must_use]
    pub fn fits(self, language: PictureLanguage) -> bool {
        self.x.is_multiple_of(2)
            && self.width.is_multiple_of(2)
            && u32::from(self.x).saturating_add(u32::from(self.width)) <= u32::from(language.width())
            && u32::from(self.y).saturating_add(u32::from(self.height)) <= u32::from(language.height())
    }
}

// A row of a picture of either language is whole bytes.
const _: () = assert!(PictureLanguage::Regis.width().is_multiple_of(2) && PictureLanguage::Naplps.width().is_multiple_of(2));

pub use interface_core::canvas::PictureLanguage;

/// A picture, or the part of one that changed (REQ-6399).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisPicture {
    /// The count of ReGIS v2 commits applied when it was painted.
    pub commits: u32,
    /// The work units its drawing cost: on the base since its stream
    /// began, and to paint its numbered drawings this time.
    pub work: u64,
    /// The language it is drawn in.
    pub language: PictureLanguage,
    /// Red, green and blue of each of the sixteen colours.
    pub palette: [[u8; 3]; REGIS_COLOURS],
    /// Which picture of its page this is: each one sent takes the next
    /// number, never 0.
    pub number: u32,
    /// The number of the picture this is a part of, to be laid over it;
    /// 0 for a whole picture, which needs none before it.
    pub base: u32,
    /// The rectangle of the picture the pixels are of: [`RegisArea::whole`]
    /// for a whole picture; for a part, where it changed, which may be
    /// nowhere.
    pub area: RegisArea,
    /// The area's pixels: row by row from its top left, two to a byte, the
    /// left one in the high half.
    pub packed: Vec<u8>,
}

impl RegisPicture {
    /// Whether it is a whole picture, not a part of one.
    #[must_use]
    pub const fn whole(&self) -> bool {
        self.base == 0
    }

    /// What both ends hold a picture to.
    fn check(&self) -> Result<(), ProtocolError> {
        if !self.area.fits(self.language) || self.packed.len() != self.area.pixels() / 2 {
            return Err(ProtocolError::BadValue("regis picture size"));
        }
        if self.number == 0 || (self.whole() && self.area != RegisArea::whole(self.language)) {
            return Err(ProtocolError::BadValue("regis picture number"));
        }
        Ok(())
    }
}

/// Where a detectable drawing's origin lies in the picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PickOrigin {
    /// The drawing, 1 to 65,535.
    pub drawing: u16,
    /// Its origin's position in the picture, which may lie outside it.
    pub x: i32,
    /// See `x`.
    pub y: i32,
}

/// A picture's pick record, in runs: row by row from the top left, each
/// run a drawing (0 for none) and how many pixels in a row it is under.
/// With no runs, no pixel has a drawing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegisPick {
    /// The origin of each drawing the runs name.
    pub origins: Vec<PickOrigin>,
    /// The runs, which cover the picture exactly, or none.
    pub runs: Vec<(u16, u16)>,
}

/// What lies under one pixel of a picture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Picked {
    /// The drawing, or 0 for none.
    pub drawing: u16,
    /// The place within the drawing; (0, 0) for none.
    pub x: i32,
    /// See `x`.
    pub y: i32,
}

/// Packs one byte a pixel into two pixels a byte. `None` unless `pixels`
/// is a whole picture drawn in `language`, of colours below sixteen.
#[must_use]
pub fn pack(pixels: &[u8], language: PictureLanguage) -> Option<Vec<u8>> {
    pack_area(pixels, RegisArea::whole(language), language)
}

/// Packs the pixels of `area` of a whole picture drawn in `language`, one
/// byte a pixel, into two pixels a byte. `None` unless `pixels` is a whole
/// picture, the area fits, and its colours are below sixteen.
#[must_use]
pub fn pack_area(pixels: &[u8], area: RegisArea, language: PictureLanguage) -> Option<Vec<u8>> {
    if pixels.len() != language.pixels() || !area.fits(language) {
        return None;
    }
    let mut packed = Vec::with_capacity(area.pixels() / 2);
    for row in usize::from(area.y)..usize::from(area.y).saturating_add(usize::from(area.height)) {
        let start = row.checked_mul(usize::from(language.width()))?.checked_add(usize::from(area.x))?;
        let line = pixels.get(start..start.checked_add(usize::from(area.width))?)?;
        if line.iter().any(|pixel| usize::from(*pixel) >= REGIS_COLOURS) {
            return None;
        }
        packed.extend(line.as_chunks::<2>().0.iter().map(|[left, right]| (left << 4) | right));
    }
    Some(packed)
}

/// Unpacks two pixels a byte into one byte a pixel, each below sixteen.
#[must_use]
pub fn unpack(packed: &[u8]) -> Vec<u8> {
    packed.iter().flat_map(|byte| [byte >> 4, byte & 0x0F]).collect()
}

impl RegisPick {
    /// A pick record in runs, from one drawing a pixel of a picture drawn
    /// in `language` and the origins of its drawings. Nothing is picked
    /// anywhere if `record` is empty (no detectable drawing wrote a pixel),
    /// is not a whole picture, or needs more runs or origins than are sent.
    #[must_use]
    pub fn of(record: &[u16], origins: impl Iterator<Item = (u16, i32, i32)>, language: PictureLanguage) -> Self {
        let origins: Vec<PickOrigin> = origins.take(MAX_PICK_ORIGINS.saturating_add(1)).map(|(drawing, x, y)| PickOrigin { drawing, x, y }).collect();
        if record.len() != language.pixels() || origins.len() > MAX_PICK_ORIGINS {
            return Self::default();
        }
        let mut runs: Vec<(u16, u16)> = Vec::new();
        for &drawing in record {
            if let Some((last, length)) = runs.last_mut()
                && *last == drawing
                && usize::from(*length) < MAX_RUN
            {
                *length = length.saturating_add(1);
            } else if runs.len() == MAX_PICK_RUNS {
                return Self::default();
            } else {
                runs.push((drawing, 1));
            }
        }
        Self { origins, runs }
    }

    /// What lies under the pixel at (`x`, `y`) of the picture drawn in
    /// `language` the record is of.
    #[must_use]
    pub fn at(&self, language: PictureLanguage, x: usize, y: usize) -> Picked {
        let width = usize::from(language.width());
        if x >= width || y >= usize::from(language.height()) {
            return Picked::default();
        }
        let mut before = y.saturating_mul(width).saturating_add(x);
        for &(drawing, length) in &self.runs {
            if before < usize::from(length) {
                let origin = self.origins.iter().find(|origin| origin.drawing == drawing);
                return match origin {
                    Some(origin) if drawing != 0 => {
                        let place = |at: usize, origin: i32| i32::try_from(at).unwrap_or(0).saturating_sub(origin);
                        Picked { drawing, x: place(x, origin.x), y: place(y, origin.y) }
                    }
                    _ => Picked::default(),
                };
            }
            before = before.saturating_sub(usize::from(length));
        }
        Picked::default()
    }
}

/// Writes a picture or a part of one.
///
/// # Errors
/// When its area does not fit the picture, its pixels are not its area's,
/// or its numbers are not a whole picture's or a part's.
pub(crate) fn push_picture(payload: &mut Vec<u8>, picture: &RegisPicture) -> Result<(), ProtocolError> {
    picture.check()?;
    payload.reserve_exact(PICTURE_FIXED.saturating_add(picture.packed.len()));
    payload.extend_from_slice(&picture.commits.to_be_bytes());
    payload.extend_from_slice(&picture.work.to_be_bytes());
    payload.push(u8::from(picture.language == PictureLanguage::Naplps));
    for colour in &picture.palette {
        payload.extend_from_slice(colour);
    }
    payload.extend_from_slice(&picture.number.to_be_bytes());
    payload.extend_from_slice(&picture.base.to_be_bytes());
    for field in [picture.area.x, picture.area.y, picture.area.width, picture.area.height] {
        payload.extend_from_slice(&field.to_be_bytes());
    }
    payload.extend_from_slice(&picture.packed);
    Ok(())
}

/// Reads a picture or a part of one. Any byte is two pixels below
/// sixteen, so the pixels need no check beyond their count.
pub(crate) fn read_picture(fields: &mut Fields<'_>) -> Result<RegisPicture, ProtocolError> {
    let commits = fields.u32()?;
    let work = u64::from_be_bytes(fields.array()?);
    let language = if fields.flag("a picture's language")? { PictureLanguage::Naplps } else { PictureLanguage::Regis };
    let mut palette = [[0_u8; 3]; REGIS_COLOURS];
    for colour in &mut palette {
        *colour = fields.array()?;
    }
    let (number, base) = (fields.u32()?, fields.u32()?);
    let area = RegisArea { x: fields.u16()?, y: fields.u16()?, width: fields.u16()?, height: fields.u16()? };
    if !area.fits(language) {
        return Err(ProtocolError::BadValue("regis picture size"));
    }
    let packed = fields.take(area.pixels() / 2)?.to_vec();
    let picture = RegisPicture { commits, work, language, palette, number, base, area, packed };
    picture.check()?;
    Ok(picture)
}

/// Writes a pick record of a picture drawn in `language`.
///
/// # Errors
/// When it has more origins or runs than are sent, or its runs do not
/// cover the picture exactly.
pub(crate) fn push_pick(payload: &mut Vec<u8>, pick: &RegisPick, language: PictureLanguage) -> Result<(), ProtocolError> {
    check_pick(pick, language)?;
    payload.extend_from_slice(&u16::try_from(pick.origins.len()).unwrap_or(0).to_be_bytes());
    for origin in &pick.origins {
        payload.extend_from_slice(&origin.drawing.to_be_bytes());
        payload.extend_from_slice(&origin.x.to_be_bytes());
        payload.extend_from_slice(&origin.y.to_be_bytes());
    }
    payload.extend_from_slice(&u16::try_from(pick.runs.len()).unwrap_or(0).to_be_bytes());
    for (drawing, length) in &pick.runs {
        payload.extend_from_slice(&drawing.to_be_bytes());
        payload.extend_from_slice(&length.to_be_bytes());
    }
    Ok(())
}

/// Reads a pick record of a picture drawn in `language`.
pub(crate) fn read_pick(fields: &mut Fields<'_>, language: PictureLanguage) -> Result<RegisPick, ProtocolError> {
    let count = fields.count_u16(MAX_PICK_ORIGINS, "too many pick origins")?;
    let mut origins = Vec::with_capacity(count);
    for _ in 0..count {
        let drawing = fields.u16()?;
        let (x, y) = (i32::from_be_bytes(fields.array()?), i32::from_be_bytes(fields.array()?));
        origins.push(PickOrigin { drawing, x, y });
    }
    let count = fields.count_u16(MAX_PICK_RUNS, "too many pick runs")?;
    let mut runs = Vec::with_capacity(count);
    for _ in 0..count {
        runs.push((fields.u16()?, fields.u16()?));
    }
    let pick = RegisPick { origins, runs };
    check_pick(&pick, language)?;
    Ok(pick)
}

/// What both ends hold a pick record of a picture drawn in `language` to.
fn check_pick(pick: &RegisPick, language: PictureLanguage) -> Result<(), ProtocolError> {
    if pick.origins.len() > MAX_PICK_ORIGINS || pick.runs.len() > MAX_PICK_RUNS {
        return Err(ProtocolError::BadValue("regis pick record size"));
    }
    if pick.origins.iter().any(|origin| origin.drawing == 0) {
        return Err(ProtocolError::BadValue("regis pick origin of no drawing"));
    }
    let covered = pick.runs.iter().fold(0_usize, |sum, (_, length)| sum.saturating_add(usize::from(*length)));
    let whole = covered == language.pixels() && pick.runs.iter().all(|(_, length)| *length != 0);
    if !pick.runs.is_empty() && !whole {
        return Err(ProtocolError::BadValue("regis pick runs do not cover the picture"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test reports a failure by panicking, and indexes data it built to a known size")]

    use super::*;

    #[test]
    fn pixels_pack_two_to_a_byte_and_unpack_as_they_were() {
        let mut pixels = vec![0_u8; REGIS_PIXELS];
        pixels[0] = 15;
        pixels[1] = 3;
        pixels[REGIS_PIXELS - 1] = 9;
        let packed = pack(&pixels, PictureLanguage::Regis).unwrap();
        assert_eq!((packed.len(), packed[0], packed[REGIS_PACKED - 1]), (REGIS_PACKED, 0xF3, 0x09));
        assert_eq!(unpack(&packed), pixels);
        pixels[5] = 16;
        assert_eq!(pack(&pixels, PictureLanguage::Regis), None, "a colour above fifteen");
        assert_eq!(pack(&pixels[1..], PictureLanguage::Regis), None, "not a whole picture");
        assert_eq!(pack(&pixels, PictureLanguage::Naplps), None, "not a NAPLPS picture's size");
    }

    #[test]
    fn a_pick_record_in_runs_finds_the_drawing_and_the_place_in_it() {
        let mut record = vec![0_u16; REGIS_PIXELS];
        for x in 100..120 {
            record[50 * REGIS_WIDTH + x] = 7;
        }
        let pick = RegisPick::of(&record, [(7, 100, 40)].into_iter(), PictureLanguage::Regis);
        assert_eq!(pick.runs.len(), 8, "runs of nothing are cut at 65,535 pixels");
        assert_eq!(pick.at(PictureLanguage::Regis, 105, 50), Picked { drawing: 7, x: 5, y: 10 });
        assert_eq!(pick.at(PictureLanguage::Regis, 99, 50), Picked::default());
        assert_eq!(pick.at(PictureLanguage::Regis, 120, 50), Picked::default());
        assert_eq!(pick.at(PictureLanguage::Regis, REGIS_WIDTH, 50), Picked::default());
        let mut bytes = Vec::new();
        push_pick(&mut bytes, &pick, PictureLanguage::Regis).unwrap();
        let mut fields = Fields::new(&bytes);
        assert_eq!(read_pick(&mut fields, PictureLanguage::Regis).unwrap(), pick);
        fields.end().unwrap();
    }

    #[test]
    fn a_pick_record_too_long_to_send_picks_nothing() {
        // Every other pixel a drawing: far more runs than are sent.
        let record: Vec<u16> = (0..REGIS_PIXELS).map(|pixel| u16::from(pixel % 2 == 0)).collect();
        assert_eq!(RegisPick::of(&record, [(1, 0, 0)].into_iter(), PictureLanguage::Regis), RegisPick::default());
        assert_eq!(RegisPick::of(&[], core::iter::empty(), PictureLanguage::Regis), RegisPick::default());
    }

    #[test]
    fn a_pick_record_that_does_not_cover_the_picture_is_refused() {
        for runs in [vec![(1_u16, 10_u16)], vec![(0, 0)]] {
            let pick = RegisPick { origins: vec![PickOrigin { drawing: 1, x: 0, y: 0 }], runs };
            assert!(push_pick(&mut Vec::new(), &pick, PictureLanguage::Regis).is_err());
        }
        let nobody = RegisPick { origins: vec![PickOrigin { drawing: 0, x: 0, y: 0 }], runs: Vec::new() };
        assert!(push_pick(&mut Vec::new(), &nobody, PictureLanguage::Regis).is_err());
        // Read from bytes: one origin of drawing 0.
        let bytes = [0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert!(read_pick(&mut Fields::new(&bytes), PictureLanguage::Regis).is_err());
    }

    /// REQ-5819: the pointer on a NAPLPS picture may be on any dot of the
    /// teletext raster, down to line 499, which no ReGIS picture has; a
    /// pixel beyond every picture is refused.
    #[test]
    fn a_pointer_may_be_on_the_rasters_last_line() {
        let key = interface_core::canvas::PageKey { channel: 1, page: 0x100 };
        for (at, accepted) in [((479, 499), true), ((799, 479), true), ((800, 0), false), ((0, 500), false)] {
            let mut bytes = Vec::new();
            push_pointer(&mut bytes, &RegisPointer { key, at: Some(at), pressed: false, finger: false });
            assert_eq!(read_pointer(&mut Fields::new(&bytes)).is_ok(), accepted, "{at:?}");
        }
    }

    /// REQ-5819, DEC-6667: a NAPLPS picture is the teletext raster, 480 by
    /// 500: it packs, is written and is read at that size, and a part or a
    /// whole of a ReGIS picture's size is refused for it, as a NAPLPS
    /// picture's size is for a ReGIS one; its pick record covers the
    /// raster's pixels.
    #[test]
    fn a_naplps_picture_is_the_teletext_raster() {
        assert_eq!((PictureLanguage::Naplps.width(), PictureLanguage::Naplps.height()), (480, 500));
        let mut pixels = vec![2_u8; PictureLanguage::Naplps.pixels()];
        pixels[479] = 5;
        let packed = pack(&pixels, PictureLanguage::Naplps).unwrap();
        assert_eq!(packed.len(), 120_000);
        let picture = |language, area: RegisArea, packed: Vec<u8>| RegisPicture {
            commits: 1,
            work: 2,
            language,
            palette: [[0; 3]; REGIS_COLOURS],
            number: 1,
            base: 0,
            area,
            packed,
        };
        let naplps = picture(PictureLanguage::Naplps, RegisArea::whole(PictureLanguage::Naplps), packed.clone());
        let mut bytes = Vec::new();
        push_picture(&mut bytes, &naplps).unwrap();
        let mut fields = Fields::new(&bytes);
        assert_eq!(read_picture(&mut fields).unwrap(), naplps);
        fields.end().unwrap();
        assert!(
            push_picture(&mut Vec::new(), &picture(PictureLanguage::Naplps, RegisArea::whole(PictureLanguage::Regis), vec![0; REGIS_PACKED])).is_err(),
            "a ReGIS size for NAPLPS"
        );
        assert!(
            push_picture(&mut Vec::new(), &picture(PictureLanguage::Regis, RegisArea::whole(PictureLanguage::Naplps), packed)).is_err(),
            "a NAPLPS size for ReGIS"
        );
        let tall = RegisArea { x: 0, y: 499, width: 2, height: 1 };
        assert!(tall.fits(PictureLanguage::Naplps) && !tall.fits(PictureLanguage::Regis), "line 499 is the raster's");
        let mut record = vec![0_u16; PictureLanguage::Naplps.pixels()];
        record[499 * 480 + 10] = 3;
        let pick = RegisPick::of(&record, [(3, 10, 499)].into_iter(), PictureLanguage::Naplps);
        assert_eq!(pick.at(PictureLanguage::Naplps, 10, 499), Picked { drawing: 3, x: 0, y: 0 });
        let mut bytes = Vec::new();
        push_pick(&mut bytes, &pick, PictureLanguage::Naplps).unwrap();
        assert_eq!(read_pick(&mut Fields::new(&bytes), PictureLanguage::Naplps).unwrap(), pick);
        assert!(read_pick(&mut Fields::new(&bytes), PictureLanguage::Regis).is_err(), "it does not cover a ReGIS picture");
    }
}

pub use interface_core::regis::RegisPointer;

/// The pointer's flag bits on the wire.
const POINTER_ON_PICTURE: u8 = 1;
const POINTER_PRESSED: u8 = 2;
const POINTER_FINGER: u8 = 4;

pub(crate) fn push_pointer(payload: &mut Vec<u8>, pointer: &RegisPointer) {
    push_key(payload, pointer.key);
    let flags = [(pointer.at.is_some(), POINTER_ON_PICTURE), (pointer.pressed, POINTER_PRESSED), (pointer.finger, POINTER_FINGER)];
    payload.push(flags.into_iter().filter(|(set, _)| *set).fold(0, |all, (_, bit)| all | bit));
    if let Some((x, y)) = pointer.at {
        payload.extend_from_slice(&x.to_be_bytes());
        payload.extend_from_slice(&y.to_be_bytes());
    }
}

pub(crate) fn read_pointer(fields: &mut Fields<'_>) -> Result<RegisPointer, ProtocolError> {
    let key = fields.key()?;
    let flags = fields.u8()?;
    if flags & !(POINTER_ON_PICTURE | POINTER_PRESSED | POINTER_FINGER) != 0 {
        return Err(ProtocolError::BadValue("a ReGIS pointer's flags"));
    }
    let at = if flags & POINTER_ON_PICTURE == 0 {
        None
    } else {
        let (x, y) = (fields.u16()?, fields.u16()?);
        // The interface is not trusted to say a pixel no picture has; the
        // trusted core holds it to the picture of its page (REQ-5819: a
        // NAPLPS picture is 480 by 500, a ReGIS picture 800 by 480).
        if x >= interface_core::canvas::PICTURE_MAX_WIDTH || y >= interface_core::canvas::PICTURE_MAX_HEIGHT {
            return Err(ProtocolError::BadValue("a ReGIS pointer outside the picture"));
        }
        Some((x, y))
    };
    Ok(RegisPointer { key, at, pressed: flags & POINTER_PRESSED != 0, finger: flags & POINTER_FINGER != 0 })
}

/// What the viewer's pointer is doing to a sub-page's picture, as the
/// trusted core tells the decoder (REQ-6374, REQ-6378): the renderer
/// paints the hovered drawing with its hover look and the pressed one with
/// its pressed look, and the decoder blinks the sub-page's cursor drawing
/// while keys go to the channel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegisViewer {
    /// The numbered drawing the pointer is over, 0 for none.
    pub hovered: u16,
    /// The numbered drawing pressed, 0 for none.
    pub pressed: u16,
    /// Keys go to the channel: the cursor drawing blinks.
    pub typing: bool,
}

pub(crate) fn push_viewer(payload: &mut Vec<u8>, viewer: RegisViewer) {
    payload.extend_from_slice(&viewer.hovered.to_be_bytes());
    payload.extend_from_slice(&viewer.pressed.to_be_bytes());
    push_flag(payload, viewer.typing);
}

pub(crate) fn read_viewer(fields: &mut Fields<'_>) -> Result<RegisViewer, ProtocolError> {
    Ok(RegisViewer { hovered: fields.u16()?, pressed: fields.u16()?, typing: fields.flag("a ReGIS viewer's typing flag")? })
}

/// The most drawing links a sub-page's picture has (the engine's REQ-6370,
/// `MAX_DRAWING_LINKS`, which the decoder asserts this equals).
pub const MAX_REGIS_LINKS: usize = 64;

/// A sub-page's drawing links as the decoder read them: each a numbered
/// drawing and the target it leads to, untrusted text the trusted core
/// parses as it parses every link's target.
#[cfg(feature = "links")]
pub(crate) fn push_links(payload: &mut Vec<u8>, links: &[(u16, String)]) -> Result<(), ProtocolError> {
    crate::push_count_u8(payload, links.len(), MAX_REGIS_LINKS, "too many drawing links")?;
    for (drawing, target) in links {
        if target.len() > crate::links::MAX_TARGET {
            return Err(ProtocolError::BadValue("a drawing link's target too long"));
        }
        payload.extend_from_slice(&drawing.to_be_bytes());
        crate::push_raw_text(payload, target, crate::links::MAX_TARGET);
    }
    Ok(())
}

#[cfg(feature = "links")]
pub(crate) fn read_links(fields: &mut Fields<'_>) -> Result<Vec<(u16, String)>, ProtocolError> {
    let count = fields.count_u8(MAX_REGIS_LINKS, "too many drawing links")?;
    (0..count).map(|_| Ok((fields.u16()?, crate::raw_text(fields, crate::links::MAX_TARGET, "a drawing link's target too long")?))).collect()
}
