//! The pick records of the pages a channel draws in ReGIS (DEC-6335): for
//! each, which numbered drawing lies under each pixel of the picture the
//! interface was last sent, so that the trusted core can say what lay
//! under a pointer (PICK, the engine's REQ-6337) without asking the
//! decoder, which may already have painted a newer picture.

use std::collections::BTreeMap;

use interface_core::canvas::PageKey;
use protocol::regis::{PictureLanguage, RegisPick};

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
    /// What the picture is drawn in, which says its size.
    language: PictureLanguage,
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

/// The pick records kept.
#[derive(Default)]
pub struct PickRecords {
    pages: BTreeMap<PageKey, Held>,
}

impl PickRecords {
    /// Keeps the pick record of the picture just sent to the interface,
    /// drawn in `language`; with none sent, the one held still stands for
    /// the same sub-page in the same language (REQ-6399 item 4), at the
    /// picture's new count of commits, and for another language nothing is
    /// picked until one comes.
    pub fn hold(&mut self, key: PageKey, sub: u16, commits: u32, language: PictureLanguage, pick: Option<RegisPick>) {
        match pick {
            Some(pick) => {
                if !self.pages.contains_key(&key) && self.pages.len() >= MAX_HELD {
                    self.pages.pop_first();
                }
                self.pages.insert(key, Held { sub, commits, language, pick });
            }
            None => {
                if let Some(held) = self.pages.get_mut(&key).filter(|held| held.sub == sub) {
                    held.commits = commits;
                    if held.language != language {
                        held.language = language;
                        held.pick = RegisPick::default();
                    }
                }
            }
        }
    }

    /// Whether `pixel` lies in the picture held for `key`, if one is: the
    /// interface is not trusted to say a pixel the picture has not
    /// (REQ-6377 item 2), and a NAPLPS picture (480 by 500, REQ-5819) and a
    /// ReGIS picture (800 by 480) have different pixels.
    #[must_use]
    pub fn has_pixel(&self, key: PageKey, pixel: (u16, u16)) -> bool {
        self.pages.get(&key).is_none_or(|held| pixel.0 < held.language.width() && pixel.1 < held.language.height())
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

    /// The sub-code of the picture held for a page, and the commits
    /// applied to it.
    #[must_use]
    pub fn held(&self, key: PageKey) -> Option<(u16, u32)> {
        self.pages.get(&key).map(|held| (held.sub, held.commits))
    }

    /// What lies under a pixel of the picture held for `key`, whichever
    /// sub-page it is of; `None` when the page is not drawn in ReGIS. The
    /// pixel is the picture's own, as the interface found it from where
    /// the picture is drawn (REQ-6377), and the reader of its message has
    /// checked that the picture has it.
    #[must_use]
    pub fn at(&self, key: PageKey, pixel: (u16, u16)) -> Option<Under> {
        let held = self.pages.get(&key)?;
        let picked = held.pick.at(held.language, usize::from(pixel.0), usize::from(pixel.1));
        Some(Under { drawing: picked.drawing, x: picked.x, y: picked.y, picture_x: u32::from(pixel.0), picture_y: u32::from(pixel.1), commits: held.commits })
    }

    /// What lies under a pixel of the picture of `key`'s sub-page `sub`;
    /// `None` when the page is not drawn in ReGIS or the picture held is
    /// of another sub-page.
    #[must_use]
    pub fn under(&self, key: PageKey, sub: u16, pixel: (u16, u16)) -> Option<Under> {
        self.held(key).filter(|(held, _)| *held == sub).and_then(|_| self.at(key, pixel))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test reports a failure by panicking, and indexes data it built to a known size")]

    use protocol::regis::{REGIS_PIXELS, REGIS_WIDTH};

    use super::*;

    const KEY: PageKey = PageKey { channel: 3, page: 0x100 };

    /// DEC-6335 item 1: the drawing under a pixel comes from the record
    /// kept with the picture shown, for the sub-page shown only.
    #[test]
    fn the_drawing_under_a_pixel_is_read_from_the_record_kept() {
        let mut record = vec![0_u16; REGIS_PIXELS];
        record[240 * REGIS_WIDTH + 400] = 9;
        let mut records = PickRecords::default();
        assert_eq!(records.under(KEY, 0, (400, 240)), None, "not a page drawn in ReGIS");
        records.hold(KEY, 0, 7, PictureLanguage::Regis, Some(RegisPick::of(&record, [(9, 390, 200)].into_iter(), PictureLanguage::Regis)));
        let under = records.under(KEY, 0, (400, 240)).unwrap();
        assert_eq!((under.drawing, under.x, under.y, under.picture_x, under.picture_y, under.commits), (9, 10, 40, 400, 240, 7));
        assert_eq!(records.under(KEY, 0, (0, 0)).unwrap().drawing, 0);
        assert_eq!(records.under(KEY, 2, (400, 240)), None, "another sub-page is on screen");
        assert_eq!(records.at(KEY, (400, 240)).map(|under| under.drawing), Some(9), "whichever sub-page the picture held is of");
        assert_eq!(records.held(KEY), Some((0, 7)));
        records.forget(KEY.channel);
        assert!(!records.has(KEY));
    }

    /// REQ-6377: a drawing one pixel wide is under its own column and not
    /// under the columns beside it. Through the page image's 480 dots,
    /// column 401 of the picture could not be named at all.
    #[test]
    fn a_drawing_one_pixel_wide_is_hit_at_its_column_and_missed_beside_it() {
        let mut record = vec![0_u16; REGIS_PIXELS];
        for row in 100..200 {
            record[row * REGIS_WIDTH + 401] = 5;
        }
        let mut records = PickRecords::default();
        records.hold(KEY, 0, 1, PictureLanguage::Regis, Some(RegisPick::of(&record, [(5, 401, 100)].into_iter(), PictureLanguage::Regis)));
        let drawing = |column: u16| records.under(KEY, 0, (column, 150)).unwrap().drawing;
        assert_eq!((drawing(400), drawing(401), drawing(402)), (0, 5, 0));
    }

    /// REQ-5819, DEC-6667: a NAPLPS page's pick record is of the teletext
    /// raster, 480 by 500: an object on line 499 is under its dot there;
    /// that dot is the picture's, where a ReGIS picture has no line 499; and
    /// a picture of another language with no record of its own picks
    /// nothing.
    #[test]
    fn a_naplps_pick_record_is_of_the_teletext_raster() {
        let width = usize::from(PictureLanguage::Naplps.width());
        let mut record = vec![0_u16; PictureLanguage::Naplps.pixels()];
        record[499 * width + 12] = 6;
        let mut records = PickRecords::default();
        assert!(records.has_pixel(KEY, (12, 499)), "no picture held: the reader's bounds hold");
        records.hold(KEY, 0, 1, PictureLanguage::Naplps, Some(RegisPick::of(&record, [(6, 12, 480)].into_iter(), PictureLanguage::Naplps)));
        let under = records.under(KEY, 0, (12, 499)).unwrap();
        assert_eq!((under.drawing, under.x, under.y), (6, 0, 19));
        assert!(records.has_pixel(KEY, (479, 499)) && !records.has_pixel(KEY, (480, 0)));
        records.hold(KEY, 0, 2, PictureLanguage::Regis, None);
        assert!(!records.has_pixel(KEY, (12, 499)) && records.has_pixel(KEY, (799, 479)), "a ReGIS picture now");
        assert_eq!(records.under(KEY, 0, (12, 10)).unwrap().drawing, 0, "nothing picked until its record comes");
    }
}
