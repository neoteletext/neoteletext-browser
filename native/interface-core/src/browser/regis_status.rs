//! What the ReGIS renderer's picture of the open page cost, drawn above
//! the picture (DEC-6335 item 4): the count of commits applied and the
//! work units its drawing took, on the base since its stream began and
//! for the last repaint of its numbered drawings. The owner
//! asked for the work units to be shown before any rule limits them.

use alloc::format;
use alloc::string::String;

use crate::browser::Browser;
use crate::canvas::{Canvas, PageKey, ROWS, Style};
use crate::geometry::Rect;
use crate::theme::GREY;

/// The text against a row's height.
const TEXT_SHARE: f64 = 0.6;
/// Room before the text, against a row's height.
const INSET_SHARE: f64 = 0.3;
/// Digits between the separators of a long number.
const GROUP: usize = 3;

/// A number with a comma between each three digits.
pub(super) fn grouped(number: u64) -> String {
    let digits = format!("{number}");
    let mut text = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && digits.len().saturating_sub(index).is_multiple_of(GROUP) {
            text.push(',');
        }
        text.push(digit);
    }
    text
}

impl Browser {
    /// The trusted core sent the picture of a page drawn in ReGIS: what it
    /// cost replaces what the last one did.
    pub fn set_regis(&mut self, key: PageKey, commits: u32, work: u64) {
        self.regis.insert(key, (commits, work));
        // Only the pages of channels still shown are kept.
        let wall = &self.wall;
        self.regis.retain(|held, _| *held == key || wall.shown().any(|(shown, _)| shown.channel == held.channel));
        self.lay_out_pictures();
    }

    /// Tells the wall which open pages take the picture's shape: those
    /// drawn in ReGIS, while the setting contains pictures in the page
    /// area; none while it contains them to the teletext page (REQ-6363).
    pub(super) fn lay_out_pictures(&mut self) {
        let contain = self.regis_contained();
        let pictures: alloc::vec::Vec<PageKey> = if contain { self.regis.keys().copied().collect() } else { alloc::vec::Vec::new() };
        self.wall.set_pictures(pictures.into_iter());
    }

    /// The cost of the open page's picture, on the image's top row, which
    /// lies above the picture.
    pub(super) fn draw_regis_status<C: Canvas + ?Sized>(&self, canvas: &mut C) {
        let Some((key, page)) = self.wall.open() else { return };
        let Some((commits, work)) = self.regis.get(&key) else { return };
        let height = page.height() / f64::from(ROWS);
        let row = Rect::new(page.x0, page.y0, page.x1, page.y0 + height);
        let text = format!("ReGIS  commit {}  {} work units", grouped(u64::from(*commits)), grouped(*work));
        #[allow(clippy::cast_possible_truncation, reason = "a row's height in logical pixels, far inside f32")]
        let style = Style { size: (height * TEXT_SHARE) as f32, monospace: true, bold: false, caps: false };
        let inset = height * INSET_SHARE;
        canvas.push_clip(row);
        canvas.text(Rect::new(row.x0 + inset, row.y0, row.x1, row.y1), &text, style, GREY);
        canvas.pop_clip();
    }
}

#[cfg(test)]
mod tests {
    use super::grouped;

    #[test]
    fn long_numbers_are_grouped_in_threes() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1_000), "1,000");
        assert_eq!(grouped(100_000_000), "100,000,000");
        assert_eq!(grouped(12_345_678), "12,345,678");
    }

    /// REQ-6363: an open page drawn in ReGIS is laid out in the picture's
    /// shape, five to three, while pictures are contained in the page
    /// area, which is the default; contained to the teletext page it keeps
    /// the teletext page's shape, as a page not drawn in ReGIS always does.
    #[test]
    fn an_open_regis_page_takes_the_pictures_shape_when_contained() {
        use crate::browser::fixture::{HEIGHT, WIDTH, browser};
        use crate::canvas::REGIS_HEIGHT_PER_WIDTH;
        let (mut browser, mut canvas) = browser();
        browser.toggle_wall(0);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let (key, teletext) = browser.wall().shown().next().unwrap();
        let shape = |rect: crate::geometry::Rect| rect.height() / rect.width();
        assert!(shape(teletext) > 1.0, "a teletext page is taller than it is wide: {teletext:?}");
        browser.set_regis(key, 0, 1);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let (_, contained) = browser.wall().shown().next().unwrap();
        assert!((shape(contained) - REGIS_HEIGHT_PER_WIDTH).abs() < 1e-9, "five to three: {contained:?}");
        assert!(contained.width() > teletext.width(), "and so wider in the same area");
        let mut settings = crate::browser::fixture::settings();
        settings.regis_contain = false;
        browser.set_settings(settings);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let (_, kept) = browser.wall().shown().next().unwrap();
        assert!((shape(kept) - shape(teletext)).abs() < 1e-9, "the teletext page's shape: {kept:?}");
    }

    /// REQ-6363: a picture contained in an area is as large as fits at its
    /// own shape and centred; in an area of its own shape, the whole area.
    #[test]
    fn a_picture_is_contained_at_its_own_shape() {
        use crate::canvas::{regis_contained, regis_within};
        use crate::geometry::Rect;
        let wide = regis_contained(Rect::new(0.0, 0.0, 1_000.0, 300.0));
        assert_eq!((wide.x0, wide.y0, wide.x1, wide.y1), (250.0, 0.0, 750.0, 300.0), "limited by the height, centred across");
        let tall = regis_contained(Rect::new(0.0, 0.0, 480.0, 500.0));
        assert_eq!((tall.x0, tall.y0, tall.x1, tall.y1), (0.0, 106.0, 480.0, 394.0), "limited by the width, centred down");
        assert_eq!(tall, regis_within(Rect::new(0.0, 0.0, 480.0, 500.0)), "in a page of the teletext shape, the teletext band");
        let own = Rect::new(10.0, 20.0, 810.0, 500.0);
        assert_eq!(regis_contained(own), own, "in its own shape, the whole area");
    }
}
