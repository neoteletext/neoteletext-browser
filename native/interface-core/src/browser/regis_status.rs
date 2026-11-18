//! What the ReGIS renderer's picture of the open page cost (DEC-6335 item
//! 4): the count of commits applied and the work units its drawing took,
//! on the base since its stream began and for the last repaint of its
//! numbered drawings, shown in the developer console's Stats tab
//! (REQ-6626) since row 0 of the page is drawn above the picture
//! (renderer parity PAR-31). The owner asked for the work units to be
//! shown before any rule limits them.

use alloc::format;
use alloc::string::String;

use crate::browser::Browser;
use crate::canvas::{Canvas, PageKey, PictureLanguage};
use crate::geometry::Rect;
use crate::theme::ROW_TITLE;

/// The label that names the size a page has come to be held at: its
/// height, the room round its words, its corners, and its distance from
/// the top of the page (REQ-6387).
const HELD_LABEL_HEIGHT: f64 = 30.0;
const HELD_LABEL_PADDING: f64 = 10.0;
const HELD_LABEL_RADIUS: f64 = 4.0;
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
    /// The trusted core sent the picture of a page drawn in ReGIS or
    /// NAPLPS: what it cost replaces what the last one did, and its
    /// language says how it is laid out. A NAPLPS picture is the teletext
    /// raster (REQ-5819, DEC-6667): its page keeps the teletext page's
    /// shape and fit, and its line above counts records and bytes, which
    /// is what a NAPLPS stream has, where a ReGIS picture has commits and
    /// work.
    pub fn set_regis(&mut self, key: PageKey, commits: u32, work: u64, language: PictureLanguage) {
        self.regis.insert(key, (commits, work));
        if language == PictureLanguage::Naplps {
            self.naplps.insert(key);
        } else {
            self.naplps.remove(&key);
        }
        // Only the pages of channels still shown are kept.
        let wall = &self.wall;
        self.regis.retain(|held, _| *held == key || wall.shown().any(|(shown, _)| shown.channel == held.channel));
        let regis = &self.regis;
        self.naplps.retain(|held| regis.contains_key(held));
        self.lay_out_pictures();
    }

    /// The language a page's picture is drawn in, if it is drawn as one.
    #[must_use]
    pub fn picture_language(&self, key: PageKey) -> Option<PictureLanguage> {
        self.regis.contains_key(&key).then(|| if self.naplps.contains(&key) { PictureLanguage::Naplps } else { PictureLanguage::Regis })
    }

    /// Tells the wall which open pages take the picture's shape: those
    /// drawn in ReGIS, while the setting contains pictures in the page
    /// area; none while it contains them to the teletext page (REQ-6363).
    /// A NAPLPS picture has the teletext page's shape, so never takes
    /// another (REQ-5819).
    pub(super) fn lay_out_pictures(&mut self) {
        let contain = self.regis_contained();
        let naplps = &self.naplps;
        let pictures: alloc::vec::Vec<PageKey> =
            if contain { self.regis.keys().copied().filter(|key| !naplps.contains(key)).collect() } else { alloc::vec::Vec::new() };
        self.wall.set_pictures(pictures.into_iter());
        let keyed = self.regis_keyed();
        self.wall.set_keyed(keyed.into_iter());
        let regis: alloc::vec::Vec<PageKey> = self.regis.keys().copied().collect();
        self.wall.set_regis_pages(regis.into_iter(), self.naplps.iter().copied());
    }

    /// The words for the multiple of its actual size a page is held at.
    fn held_words(multiple: u32) -> String {
        match multiple {
            1 => "Actual size".into(),
            2 => "Twice actual size".into(),
            more => format!("{more} times actual size"),
        }
    }

    /// When the label that names the size the open page came to be held at
    /// goes, while it shows: a frame is drawn then, without it.
    pub(super) fn held_label_deadline(&self, now: u64) -> Option<u64> {
        self.wall.held_label(now).map(|(_, until)| until.saturating_sub(now).max(1))
    }

    /// The label over the top of the open page for a moment after it comes
    /// to be held at a whole multiple of its actual size (REQ-6387): "Actual
    /// size", in the browser's own type.
    pub(super) fn draw_held_label<C: Canvas + ?Sized>(&self, canvas: &mut C, now: u64) {
        let Some((multiple, _)) = self.wall.held_label(now) else { return };
        let Some((_, image)) = self.wall.open() else { return };
        let words = Self::held_words(multiple);
        let width = canvas.text_width(&words, ROW_TITLE) + HELD_LABEL_PADDING * 2.0;
        let left = (image.x0 + image.x1 - width) / 2.0;
        let place = Rect::new(left, image.y0 + HELD_LABEL_PADDING, left + width, image.y0 + HELD_LABEL_PADDING + HELD_LABEL_HEIGHT);
        canvas.fill(place, HELD_LABEL_RADIUS, crate::theme::BUTTON);
        crate::text::centred(canvas, place, &words, ROW_TITLE, crate::theme::WHITE);
    }

    /// The pages drawn in ReGIS whose teletext page beneath has
    /// coloured-key prompts: each shows that row beneath its picture
    /// (REQ-6379).
    #[must_use]
    pub fn regis_keyed(&self) -> alloc::vec::Vec<PageKey> {
        #[cfg(feature = "links")]
        return self.regis.keys().copied().filter(|key| self.has_prompts(*key)).collect();
        #[cfg(not(feature = "links"))]
        alloc::vec::Vec::new()
    }

    /// What the open page's picture has cost, for the developer console's
    /// Stats tab (REQ-6626): the commits and work units of a ReGIS picture,
    /// the records and bytes of a NAPLPS one. It was drawn over the page's
    /// top row, which is now the page's own row 0 (renderer parity PAR-31).
    #[must_use]
    pub(super) fn picture_cost(&self) -> Option<crate::console::Stat> {
        let (key, _) = self.wall.open()?;
        let (commits, work) = self.regis.get(&key)?;
        Some(if self.naplps.contains(&key) {
            crate::console::Stat { label: "NAPLPS picture".into(), value: format!("record {}, {} bytes", grouped(u64::from(*commits)), grouped(*work)) }
        } else {
            crate::console::Stat { label: "ReGIS picture".into(), value: format!("commit {}, {} work units", grouped(u64::from(*commits)), grouped(*work)) }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::grouped;
    use crate::canvas::PictureLanguage;

    #[test]
    fn long_numbers_are_grouped_in_threes() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1_000), "1,000");
        assert_eq!(grouped(100_000_000), "100,000,000");
        assert_eq!(grouped(12_345_678), "12,345,678");
    }

    /// REQ-6363: an open page drawn in ReGIS is laid out in the picture's
    /// shape, five to three, with its teletext page's row 0 above it
    /// (renderer parity PAR-31), while pictures are contained in the page
    /// area, which is the default; contained to the teletext page it keeps
    /// the teletext page's shape, as a page not drawn in ReGIS always does.
    #[test]
    fn an_open_regis_page_takes_the_pictures_shape_when_contained() {
        use crate::browser::fixture::{HEIGHT, WIDTH, browser};
        use crate::canvas::REGIS_HEADED_HEIGHT_PER_WIDTH;
        let (mut browser, mut canvas) = browser();
        browser.toggle_wall(0);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let (key, teletext) = browser.wall().shown().next().unwrap();
        let shape = |rect: crate::geometry::Rect| rect.height() / rect.width();
        assert!(shape(teletext) > 1.0, "a teletext page is taller than it is wide: {teletext:?}");
        browser.set_regis(key, 0, 1, PictureLanguage::Regis);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let (_, contained) = browser.wall().shown().next().unwrap();
        assert!((shape(contained) - REGIS_HEADED_HEIGHT_PER_WIDTH).abs() < 1e-9, "five to three, and row 0: {contained:?}");
        assert!(contained.width() > teletext.width(), "and so wider in the same area");
        let mut settings = crate::browser::fixture::settings();
        settings.regis_contain = false;
        browser.set_settings(settings);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let (_, kept) = browser.wall().shown().next().unwrap();
        assert!((shape(kept) - shape(teletext)).abs() < 1e-9, "the teletext page's shape: {kept:?}");
    }

    /// REQ-6387: the label over a page that has come to be held at its
    /// actual size says "Actual size", never a number and a cross; it is
    /// drawn over the top of the page, and is gone a second and a half
    /// later, for which a frame is asked.
    #[test]
    fn the_held_label_says_actual_size_and_goes() {
        use crate::browser::Browser;
        use crate::browser::fixture::{HEIGHT, WIDTH, browser};
        use crate::canvas::STICK_LABEL_MILLISECONDS;
        use crate::test_canvas::Recorder;
        assert_eq!(Browser::held_words(1), "Actual size");
        assert_eq!(Browser::held_words(2), "Twice actual size");
        assert_eq!(Browser::held_words(3), "3 times actual size");
        let (mut browser, _) = browser();
        browser.toggle_wall(0);
        // The window's width at which the open page's room is a little
        // past a whole multiple is found, not assumed: the bars and panels around
        // the page is not this test's business.
        let found = (0..2_000_u32).map(|step| WIDTH + f64::from(step)).find(|width| {
            let (mut fresh, _) = crate::browser::fixture::browser();
            fresh.toggle_wall(0);
            fresh.draw(&mut Recorder::new(), *width, HEIGHT * 4.0, 0);
            fresh.wall().held_label(0).is_some()
        });
        let width = found.expect("some width holds the page");
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, width, HEIGHT * 4.0, 0);
        let (multiple, _) = browser.wall().held_label(0).unwrap();
        let words = Browser::held_words(multiple);
        let (_, image) = browser.wall().open().unwrap();
        let (place, _) = canvas.texts.iter().find(|(_, text)| *text == words).expect("the label is drawn");
        assert!(place.y0 >= image.y0 && place.y1 <= image.y1 && place.x0 >= image.x0 && place.x1 <= image.x1, "over the page: {place:?} in {image:?}");
        let crossed =
            |text: &str| text.strip_suffix(['x', '\u{d7}']).is_some_and(|number| !number.is_empty() && number.chars().all(|digit| digit.is_ascii_digit()));
        assert!(!canvas.texts.iter().any(|(_, text)| crossed(text)), "no number and cross");
        assert_eq!(browser.held_label_deadline(0), Some(STICK_LABEL_MILLISECONDS));
        let mut later = Recorder::new();
        browser.draw(&mut later, width, HEIGHT * 4.0, STICK_LABEL_MILLISECONDS);
        assert!(!later.texts.iter().any(|(_, text)| *text == words), "gone");
        assert_eq!(browser.held_label_deadline(STICK_LABEL_MILLISECONDS), None);
    }

    /// DEC-6407: a picture drawn in NAPLPS has its cost named NAPLPS, with
    /// its records and bytes; one drawn in ReGIS, its commits and work.
    /// Renderer parity PAR-31: it is for the Stats tab, and nothing of it is
    /// drawn over the page, whose row 0 is above the picture.
    #[test]
    fn a_pictures_cost_names_its_language_and_is_not_drawn_over_the_page() {
        use crate::browser::fixture::{HEIGHT, WIDTH, browser};
        let (mut browser, mut canvas) = browser();
        browser.toggle_wall(0);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let (key, _) = browser.wall().shown().next().unwrap();
        browser.set_regis(key, 3, 1_234, PictureLanguage::Regis);
        let texts = |browser: &mut crate::browser::Browser| {
            let mut canvas = crate::test_canvas::Recorder::new();
            browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
            canvas.texts.into_iter().map(|(_, text)| text).collect::<alloc::vec::Vec<_>>()
        };
        let cost = |browser: &crate::browser::Browser| browser.picture_cost().map(|stat| (stat.label, stat.value));
        assert_eq!(cost(&browser), Some(("ReGIS picture".into(), "commit 3, 1,234 work units".into())));
        assert!(!texts(&mut browser).iter().any(|text| text.contains("work units")), "nothing drawn over the page");
        browser.set_regis(key, 3, 1_234, PictureLanguage::Naplps);
        assert_eq!(cost(&browser), Some(("NAPLPS picture".into(), "record 3, 1,234 bytes".into())));
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

#[cfg(test)]
mod key_row_tests {
    use crate::canvas::{REGIS_HEIGHT_PER_WIDTH, ROW_HEIGHT_PER_WIDTH, ROWS, regis_contained, regis_layout, regis_within};
    use crate::geometry::{Point, Rect};
    use crate::wall::cell_in;

    fn close(one: f64, other: f64) -> bool {
        (one - other).abs() < 1e-9
    }

    /// REQ-6379 item 3: contained in the page area, the row of prompts
    /// lies directly beneath the picture, as wide as it and a teletext
    /// row's height for that width, one twenty-fourth; the picture keeps
    /// its shape; the two together are as large as fits, centred.
    #[test]
    fn the_row_of_prompts_lies_directly_beneath_a_contained_picture() {
        for area in [Rect::new(0.0, 0.0, 800.0, 480.0 + 800.0 / 24.0), Rect::new(10.0, 20.0, 1500.0, 700.0), Rect::new(0.0, 0.0, 300.0, 900.0)] {
            let layout = regis_layout(area, true, true);
            let (header, picture, keys) = (layout.header, layout.picture, layout.keys.unwrap());
            assert!(layout.stacked);
            assert!(close(picture.height() / picture.width(), REGIS_HEIGHT_PER_WIDTH), "{area:?}");
            assert!(close(header.y1, picture.y0) && close(header.height(), keys.height()) && close(header.x0, picture.x0), "row 0 directly above: {area:?}");
            assert!(close(keys.y0, picture.y1) && close(keys.x0, picture.x0) && close(keys.x1, picture.x1), "{area:?}");
            assert!(close(keys.height(), picture.width() * ROW_HEIGHT_PER_WIDTH) && close(keys.height(), picture.width() / 24.0), "{area:?}");
            assert!(picture.x0 >= area.x0 - 1e-9 && keys.y1 <= area.y1 + 1e-9, "inside the area: {area:?}");
            assert!(close(picture.width(), area.width()) || close(keys.y1 - header.y0, area.height()), "as large as fits: {area:?}");
        }
    }

    /// With no prompts the picture is placed with row 0 alone above it
    /// (PAR-31); contained to the teletext page, the picture is in its
    /// band, row 0 and the row of prompts where the teletext page has its
    /// rows 0 and 24 (REQ-6379 items 1 and 4).
    #[test]
    fn without_prompts_only_row_0_is_above_and_in_the_teletext_fit_the_rows_are_rows_0_and_24() {
        use crate::canvas::REGIS_HEADED_HEIGHT_PER_WIDTH;
        let area = Rect::new(5.0, 7.0, 485.0, 507.0);
        let plain = regis_layout(area, true, false);
        assert_eq!(plain.keys, None);
        assert!(close(plain.header.y1, plain.picture.y0) && close((plain.picture.y1 - plain.header.y0) / plain.picture.width(), REGIS_HEADED_HEIGHT_PER_WIDTH));
        assert!(close(plain.picture.width(), regis_contained(area).width()), "as wide as the picture contained alone, in this wide area");
        let teletext_header = regis_layout(area, false, false).header;
        assert!(close(teletext_header.y0, area.y0) && close(teletext_header.height(), area.height() / f64::from(ROWS)), "row 0 where the teletext page has it");
        assert_eq!(regis_layout(area, false, false).picture, regis_within(area));
        let teletext = regis_layout(area, false, true);
        assert_eq!(teletext.picture, regis_within(area));
        assert!(!teletext.stacked);
        let keys = teletext.keys.unwrap();
        assert!(close(keys.y1, area.y1) && close(keys.height(), area.height() / f64::from(ROWS)));
        // The teletext page's own grid finds row 24 there.
        let within = Point::new(area.x0 + 100.0, keys.y0 + 1.0);
        assert_eq!(teletext.grid_point(area, within), Some(within));
        assert_eq!(cell_in(area, within).map(|(_, row)| row), Some(24));
    }

    /// Renderer parity PAR-31: a NAPLPS picture, the teletext raster, has
    /// row 0 over the raster's lines 0 to 19 and the row of prompts over
    /// lines 480 to 499, where the teletext page has them, in either fit.
    #[test]
    fn a_naplps_pictures_row_0_and_row_24_lie_on_the_rasters_first_and_last_twenty_lines() {
        use crate::canvas::{PictureLanguage, picture_layout};
        let area = Rect::new(0.0, 0.0, 480.0, 500.0);
        for contain in [true, false] {
            let layout = picture_layout(PictureLanguage::Naplps, area, contain, true);
            assert_eq!(layout.picture, area, "the raster fills the image");
            assert_eq!(layout.header, Rect::new(0.0, 0.0, 480.0, 20.0), "row 0: lines 0 to 19");
            assert_eq!(layout.keys, Some(Rect::new(0.0, 480.0, 480.0, 500.0)), "row 24: lines 480 to 499");
            assert!(!layout.stacked);
        }
    }

    /// REQ-6379 items 6 and 7: under a contained picture, a point in the
    /// row of prompts is on row 24 of the teletext page beneath, at its
    /// column; row 0 is above the picture (PAR-31), which stands for rows 1
    /// to 23; a point beside them is
    /// on no cell.
    #[test]
    fn a_point_in_the_row_of_prompts_is_on_row_24_of_the_page_beneath() {
        let area = Rect::new(0.0, 0.0, 1000.0, 1000.0);
        let layout = regis_layout(area, true, true);
        let (picture, keys) = (layout.picture, layout.keys.unwrap());
        let cell = |point: Point| layout.grid_point(area, point).and_then(|point| cell_in(area, point));
        assert_eq!(cell(Point::new(keys.x0 + 1.0, keys.y0 + 0.5)), Some((0, 24)), "the row's first line");
        assert_eq!(cell(Point::new(keys.x1 - 1.0, keys.y1 - 0.5)), Some((39, 24)), "and its last");
        assert_eq!(cell(Point::new(picture.x0 + 1.0, picture.y1 - 0.5)), Some((0, 23)), "the picture's last line is row 23");
        assert_eq!(cell(Point::new(picture.x0 + picture.width() / 2.0, picture.y0 + 0.5)), Some((20, 1)), "the picture's first line is row 1");
        let header = layout.header;
        assert_eq!(cell(Point::new(header.x0 + 1.0, header.y0 + 0.5)), Some((0, 0)), "row 0 above it (PAR-31)");
        assert_eq!(cell(Point::new(header.x1 - 1.0, header.y1 - 0.5)), Some((39, 0)));
        assert_eq!(cell(Point::new(picture.x0 + 1.0, header.y0 - 5.0)), None, "above row 0, within the area");
    }
}
