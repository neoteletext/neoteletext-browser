//! The pointer on a page drawn in ReGIS (REQ-6377): which pixel of the
//! picture it is over, found from the rectangle the picture is drawn in.
//!
//! A picture is 800 by 480 pixels and its page's image 480 dots across, so
//! a place found through the image's dots can name only three of every
//! five columns. Every lookup in the picture's pick record (what a click
//! lands on, what the pointer is over) uses the pixel found here.

use crate::canvas::{PageKey, PictureLanguage};
use crate::geometry::{Point, Rect};

/// Half of a cursor's blink, in ticks of the presentation clock, of which
/// there are sixty a second (neoteletext-common's `docs/animation.md`
/// ANIM-1): 32 ticks, 533 milliseconds, about the half period common
/// terminals blink at. It is here, outside the typing code, because the
/// decoder blinks the cursor drawing of a ReGIS page at the same period
/// (REQ-6378), with or without typing compiled in.
pub const BLINK_HALF_TICKS: u32 = 32;
/// [`BLINK_HALF_TICKS`] on the interface's clock of milliseconds.
pub const BLINK_MILLISECONDS: u64 = neoteletext_core::consts::presentation_tick_due_us(BLINK_HALF_TICKS) / MICROSECONDS_PER_MILLISECOND;
const MICROSECONDS_PER_MILLISECOND: u64 = 1_000;
const _: () = assert!(BLINK_MILLISECONDS == 533);

/// Where the pointer is on a page drawn in ReGIS, as the interface tells
/// the trusted core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegisPointer {
    /// The open page.
    pub key: PageKey,
    /// The pixel of the picture under the pointer, in the picture's own
    /// pixels ([`PictureLanguage::width`] by [`PictureLanguage::height`]);
    /// `None` when the pointer is off the picture.
    pub at: Option<(u16, u16)>,
    /// The main button, or the finger, is down.
    pub pressed: bool,
    /// It is a finger, which presses and does not hover.
    pub finger: bool,
}

/// The pixel of a picture drawn in `language` in `rect` that lies under
/// `point`, or `None` when the point is outside the rectangle. Shown at its own size
/// or larger, every pixel of the picture can be pointed at; shown smaller,
/// the pixels that share a screen pixel are named by the first of them.
#[must_use]
pub fn pixel_under(language: PictureLanguage, rect: Rect, point: Point) -> Option<(u16, u16)> {
    let (width, height) = (language.width(), language.height());
    if !rect.contains(point) || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return None;
    }
    // Multiplied before dividing, so that a whole number of pixels comes
    // out whole.
    let across = (point.x - rect.x0) * f64::from(width) / rect.width();
    let down = (point.y - rect.y0) * f64::from(height) / rect.height();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "within the rectangle, as checked, so from 0 to the picture's size")]
    let (across, down) = (libm::floor(across) as u16, libm::floor(down) as u16);
    // The far edges belong to the last pixel.
    Some((across.min(width.saturating_sub(1)), down.min(height.saturating_sub(1))))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// REQ-6377: in a picture shown at twice its size every pixel pair of
    /// the screen maps to its picture pixel, and every one of the 800
    /// columns and 480 rows is reached.
    #[test]
    fn every_pixel_of_a_picture_shown_larger_can_be_pointed_at() {
        let rect = Rect::new(100.0, 50.0, 1700.0, 1010.0);
        let mut columns = alloc::collections::BTreeSet::new();
        let mut rows = alloc::collections::BTreeSet::new();
        for step in 0..1600_u32 {
            let (x, _) = pixel_under(PictureLanguage::Regis, rect, Point::new(100.0 + f64::from(step) + 0.5, 60.0)).unwrap();
            assert_eq!(u32::from(x), step / 2);
            columns.insert(x);
        }
        for step in 0..960_u32 {
            let (_, y) = pixel_under(PictureLanguage::Regis, rect, Point::new(110.0, 50.0 + f64::from(step) + 0.5)).unwrap();
            assert_eq!(u32::from(y), step / 2);
            rows.insert(y);
        }
        assert_eq!((columns.len(), rows.len()), (800, 480));
    }

    /// The corners are the picture's corners, and a point outside the
    /// rectangle is on no pixel.
    #[test]
    fn the_corners_are_the_pictures_and_outside_is_nothing() {
        let rect = Rect::new(10.0, 20.0, 410.0, 260.0);
        assert_eq!(pixel_under(PictureLanguage::Regis, rect, Point::new(10.0, 20.0)), Some((0, 0)));
        assert_eq!(pixel_under(PictureLanguage::Regis, rect, Point::new(409.9, 259.9)), Some((799, 479)));
        assert_eq!(pixel_under(PictureLanguage::Regis, rect, Point::new(9.9, 100.0)), None);
        assert_eq!(pixel_under(PictureLanguage::Regis, rect, Point::new(100.0, 260.5)), None);
    }

    /// Shown at half its size, a screen pixel names the first of the
    /// picture pixels that share it.
    #[test]
    fn a_picture_shown_smaller_names_the_first_pixel_under_each_screen_pixel() {
        let rect = Rect::new(0.0, 0.0, 400.0, 240.0);
        assert_eq!(pixel_under(PictureLanguage::Regis, rect, Point::new(200.0, 120.0)), Some((400, 240)));
        assert_eq!(pixel_under(PictureLanguage::Regis, rect, Point::new(201.0, 121.0)), Some((402, 242)));
    }

    /// REQ-5819, DEC-6667: a NAPLPS picture is the teletext raster, so a
    /// page image drawn at its own size names each dot of the raster, and
    /// its far corner is the raster's last pixel, 479 by 499: the cell of
    /// a dot is the teletext page's cell there.
    #[test]
    fn a_naplps_picture_is_pointed_at_in_the_rasters_dots() {
        let rect = Rect::new(0.0, 0.0, 480.0, 500.0);
        assert_eq!(pixel_under(PictureLanguage::Naplps, rect, Point::new(12.5, 20.5)), Some((12, 20)), "the first dot of cell (1, 1)");
        assert_eq!(pixel_under(PictureLanguage::Naplps, rect, Point::new(479.9, 499.9)), Some((479, 499)));
        let twice = Rect::new(0.0, 0.0, 960.0, 1000.0);
        assert_eq!(pixel_under(PictureLanguage::Naplps, twice, Point::new(241.0, 39.0)), Some((120, 19)), "the display area's first line");
    }
}
