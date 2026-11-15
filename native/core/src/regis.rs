//! The pick records of the pages a channel draws in ReGIS (DEC-6335): for
//! each, which numbered drawing lies under each pixel of the picture the
//! interface was last sent, so that the trusted core can say what lay
//! under a pointer (PICK, the engine's REQ-6337) without asking the
//! decoder, which may already have painted a newer picture.

use std::collections::BTreeMap;

use interface_core::canvas::{PAGE_HEIGHT, PAGE_WIDTH, PageKey, REGIS_LINES, REGIS_TOP_LINE};
use protocol::regis::{REGIS_HEIGHT, REGIS_WIDTH, RegisPick};

/// The most pages whose pick record is kept: more than a window shows of
/// one channel at once. One more forgets the lowest page of the lowest
/// channel, whose record comes again with its next picture.
pub const MAX_HELD: usize = 16;

/// One page's pick record.
struct Held {
    /// The sub-code the picture is of.
    sub: u16,
    /// The commits applied to the picture.
    commits: u32,
    pick: RegisPick,
}

/// What lay under a dot of a page drawn in ReGIS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Under {
    /// The numbered drawing, or 0 for none.
    pub drawing: u16,
    /// The place within it; (0, 0) for none.
    pub x: i32,
    /// See `x`.
    pub y: i32,
    /// The place in the picture, in its own pixels.
    pub picture_x: u32,
    /// See `picture_x`.
    pub picture_y: u32,
    /// The commits applied to the picture.
    pub commits: u32,
}

/// The pixel of a ReGIS picture under a dot of its page's image, or `None`
/// for a dot above or below the picture. The picture is drawn across the
/// whole width of the image and [`REGIS_LINES`] of its lines, from line
/// [`REGIS_TOP_LINE`] (`interface_core::canvas::regis_within`).
#[must_use]
pub fn picture_pixel(dot: (u32, u32)) -> Option<(usize, usize)> {
    let (x, y) = (usize::try_from(dot.0).ok()?, usize::try_from(dot.1).ok()?);
    let line = y.checked_sub(usize::from(REGIS_TOP_LINE)).filter(|line| *line < usize::from(REGIS_LINES))?;
    if x >= usize::from(PAGE_WIDTH) || y >= usize::from(PAGE_HEIGHT) {
        return None;
    }
    let across = x.saturating_mul(REGIS_WIDTH).checked_div(usize::from(PAGE_WIDTH))?;
    let down = line.saturating_mul(REGIS_HEIGHT).checked_div(usize::from(REGIS_LINES))?;
    Some((across.min(REGIS_WIDTH.saturating_sub(1)), down.min(REGIS_HEIGHT.saturating_sub(1))))
}

/// The pick records kept.
#[derive(Default)]
pub struct PickRecords {
    pages: BTreeMap<PageKey, Held>,
}

impl PickRecords {
    /// Keeps the pick record of the picture just sent to the interface.
    pub fn hold(&mut self, key: PageKey, sub: u16, commits: u32, pick: RegisPick) {
        if !self.pages.contains_key(&key) && self.pages.len() >= MAX_HELD {
            self.pages.pop_first();
        }
        self.pages.insert(key, Held { sub, commits, pick });
    }

    /// Whether a page is drawn in ReGIS.
    #[must_use]
    pub fn has(&self, key: PageKey) -> bool {
        self.pages.contains_key(&key)
    }

    /// Forgets a channel's pages: it closed.
    pub fn forget(&mut self, channel: u32) {
        self.pages.retain(|key, _| key.channel != channel);
    }

    /// What lies under a dot of the image of `key`'s sub-page `sub`; `None`
    /// when the page is not drawn in ReGIS, another sub-page of it is, or
    /// the dot is outside the picture.
    #[must_use]
    pub fn under(&self, key: PageKey, sub: u16, dot: (u32, u32)) -> Option<Under> {
        let held = self.pages.get(&key).filter(|held| held.sub == sub)?;
        let (x, y) = picture_pixel(dot)?;
        let picked = held.pick.at(x, y);
        Some(Under {
            drawing: picked.drawing,
            x: picked.x,
            y: picked.y,
            picture_x: u32::try_from(x).ok()?,
            picture_y: u32::try_from(y).ok()?,
            commits: held.commits,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test reports a failure by panicking, and indexes data it built to a known size")]

    use protocol::regis::REGIS_PIXELS;

    use super::*;

    const KEY: PageKey = PageKey { channel: 3, page: 0x100 };

    /// The picture fills the image's width and 288 of its 500 lines, from
    /// line 106: the corners of that band are the picture's corners, and
    /// the lines above and below are outside it.
    #[test]
    fn a_dot_of_the_image_is_a_pixel_of_the_picture_or_outside_it() {
        assert_eq!(picture_pixel((0, 106)), Some((0, 0)));
        assert_eq!(picture_pixel((479, 393)), Some((798, 478)));
        assert_eq!(picture_pixel((240, 250)), Some((400, 240)));
        assert_eq!(picture_pixel((0, 105)), None);
        assert_eq!(picture_pixel((0, 394)), None);
        assert_eq!(picture_pixel((480, 200)), None);
    }

    /// DEC-6335 item 1: the drawing under a dot comes from the record kept
    /// with the picture shown, for the sub-page shown only.
    #[test]
    fn the_drawing_under_a_dot_is_read_from_the_record_kept() {
        let mut record = vec![0_u16; REGIS_PIXELS];
        record[240 * REGIS_WIDTH + 400] = 9;
        let mut records = PickRecords::default();
        assert_eq!(records.under(KEY, 0, (240, 250)), None, "not a page drawn in ReGIS");
        records.hold(KEY, 0, 7, RegisPick::of(&record, [(9, 390, 200)].into_iter()));
        let under = records.under(KEY, 0, (240, 250)).unwrap();
        assert_eq!((under.drawing, under.x, under.y, under.picture_x, under.picture_y, under.commits), (9, 10, 40, 400, 240, 7));
        assert_eq!(records.under(KEY, 0, (0, 106)).unwrap().drawing, 0);
        assert_eq!(records.under(KEY, 2, (240, 250)), None, "another sub-page is on screen");
        assert_eq!(records.under(KEY, 0, (240, 10)), None, "above the picture");
        records.forget(KEY.channel);
        assert!(!records.has(KEY));
    }
}
