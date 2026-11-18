//! The pointer on the open page's ReGIS picture (REQ-6377, REQ-6374): the
//! trusted core is told the pixel of the picture under it and whether it
//! is pressed, and says back whether a drawing a click would act on lies
//! there, which makes the pointer the hand. What is drawn where is the
//! trusted core's knowledge alone.

use super::Browser;
use super::output::Output;
use crate::canvas::{PageKey, PictureLanguage, picture_layout};
use crate::geometry::{Point, Rect};
use crate::regis::{RegisPointer, pixel_under, span_of};

/// The open page, the pixel of its picture under a point if any, and how
/// many of the picture's pixels one screen pixel covers there.
type PointedPixel = (PageKey, Option<(u16, u16)>, u8);

/// What the interface has said of the pointer, and been told.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct RegisPointing {
    /// What the trusted core was last told.
    sent: Option<RegisPointer>,
    /// The main button is down, having gone down on the picture.
    pressed: bool,
    /// A finger is down on the picture, and where.
    finger: Option<Point>,
    /// The page on whose picture the trusted core says a drawing lies
    /// under the pointer.
    over: Option<PageKey>,
}

impl Browser {
    /// The open page, if it is drawn in ReGIS or NAPLPS, and the pixel of
    /// its picture under `point`, if any: the picture is drawn in the
    /// page's image as the painter places it, by the fit in force
    /// (REQ-6363). A NAPLPS picture is the teletext raster (REQ-5819), so
    /// its pixel is the dot of the page image, in the cell the teletext
    /// page has there.
    fn regis_pixel(&self, point: Point) -> Option<PointedPixel> {
        let (key, language, picture) = self.open_picture()?;
        Some((key, pixel_under(language, picture, point), span_of(language, picture)))
    }

    /// The open page, if it is drawn as a picture, the picture's language,
    /// and where the picture is drawn, by the fit in force (REQ-6363).
    pub(super) fn open_picture(&self) -> Option<(PageKey, PictureLanguage, Rect)> {
        let (key, image) = self.wall.open()?;
        let language = self.picture_language(key)?;
        let keyed = self.regis_keyed().contains(&key);
        Some((key, language, picture_layout(language, image, self.regis_contained(), keyed).picture))
    }

    /// The pointer pressed (`true`) or let go on `pixel` of the open page's
    /// picture, as the keyboard does it on a focused picture target that is
    /// not a link (renderer parity PAR-30): the trusted core is told that
    /// pixel, one picture pixel across, as a pointer there would tell it.
    #[cfg(feature = "interaction")]
    pub(super) fn regis_press_at(&mut self, key: PageKey, pixel: (u16, u16), pressed: bool) {
        let pointer = RegisPointer { key, at: Some(pixel), pressed, finger: false, span: 1 };
        self.outputs.push(Output::RegisPointer(pointer));
        self.regis_pointer.sent = Some(pointer);
    }

    /// Tells the trusted core where the pointer is on the open page's
    /// picture, when that or its press has changed since it was last told;
    /// and that it has left, when the page is no longer open or drawn in
    /// ReGIS. Called before any input of the same moment is sent to the
    /// channel, so that what the trusted core looks up for that input is
    /// of this place.
    pub(super) fn regis_pointed(&mut self) {
        let finger = self.regis_pointer.finger;
        let wanted = self.regis_pixel(finger.unwrap_or(self.pointer)).map(|(key, at, span)| RegisPointer {
            key,
            at,
            pressed: at.is_some() && (self.regis_pointer.pressed || finger.is_some()),
            finger: finger.is_some(),
            span,
        });
        let sent = self.regis_pointer.sent;
        if wanted == sent {
            return;
        }
        // Gone from a page it was on: that page's pointer is off its picture.
        if let Some(sent) = sent.filter(|sent| wanted.is_none_or(|wanted| wanted.key != sent.key) && sent.at.is_some()) {
            self.outputs.push(Output::RegisPointer(RegisPointer { key: sent.key, at: None, pressed: false, finger: false, span: 1 }));
        }
        if let Some(wanted) = wanted {
            self.outputs.push(Output::RegisPointer(wanted));
        } else {
            self.regis_pointer.over = None;
        }
        self.regis_pointer.sent = wanted;
    }

    /// The main button went down or came up.
    pub(super) fn regis_button(&mut self, down: bool) {
        self.regis_pointer.pressed = down && self.regis_pixel(self.pointer).is_some_and(|(_, at, _)| at.is_some());
        self.regis_pointed();
    }

    /// A finger came down or moved (`Some`), or lifted (`None`). Touches
    /// reach the browser only with typing built in.
    #[cfg(feature = "interaction")]
    pub(super) fn regis_finger(&mut self, at: Option<Point>) {
        self.regis_pointer.finger = at;
        self.regis_pointed();
    }

    /// The trusted core says whether a drawing a click would act on lies
    /// under the pointer on a page's picture.
    pub fn set_regis_over(&mut self, key: PageKey, over: bool) {
        self.regis_pointer.over = over.then_some(key);
    }

    /// The pointer's shape over a drawing a click would act on: the hand,
    /// or the shape for a link that leaves the channel while the strip
    /// shows such a destination, as over a text link (REQ-6376 item 3).
    #[cfg_attr(not(feature = "links"), expect(clippy::unused_self, reason = "only a link that leaves the channel reads the browser"))]
    pub(super) fn regis_cursor(&self) -> super::Cursor {
        #[cfg(feature = "links")]
        if self.pointing_leaves() {
            return super::Cursor::External;
        }
        super::Cursor::Hand
    }

    /// Whether the pointer is over a drawing of the open page's picture
    /// that a click would act on: it is then the hand.
    pub(super) fn over_regis_drawing(&self) -> bool {
        let over = self.regis_pointer.over;
        over.is_some() && self.regis_pixel(self.pointer).is_some_and(|(key, at, _)| Some(key) == over && at.is_some())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a test reports a failure by panicking")]

    use alloc::vec::Vec;

    use super::*;
    use crate::browser::Cursor;
    use crate::browser::fixture::{HEIGHT, WIDTH, browser};
    use crate::geometry::Rect;

    /// A browser with its one page open and drawn in ReGIS, and the
    /// rectangle the picture is drawn in.
    fn open_picture(contain: bool) -> (Browser, PageKey, Rect) {
        let (mut browser, mut canvas) = browser();
        let mut settings = crate::browser::fixture::settings();
        settings.regis_contain = contain;
        browser.set_settings(settings);
        browser.toggle_wall(0);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let (key, _) = browser.wall().shown().next().unwrap();
        browser.set_regis(key, 0, 1, PictureLanguage::Regis);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let (_, image) = browser.wall.open().unwrap();
        let picture = picture_layout(PictureLanguage::Regis, image, contain, false).picture;
        let _ = browser.take_outputs();
        (browser, key, picture)
    }

    /// The middle of a rectangle.
    fn middle_of(rect: Rect) -> Point {
        Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1))
    }

    /// The pointers the trusted core was told of since they were last taken.
    fn told_with_spans(browser: &mut Browser) -> Vec<RegisPointer> {
        browser.take_outputs().into_iter().filter_map(|output| if let Output::RegisPointer(pointer) = output { Some(pointer) } else { None }).collect()
    }

    /// The same, each span read as one: the tests of the pixel found leave
    /// the span to [`the_span_is_the_picture_pixels_a_screen_pixel_covers`].
    fn told(browser: &mut Browser) -> Vec<RegisPointer> {
        told_with_spans(browser).into_iter().map(|pointer| RegisPointer { span: 1, ..pointer }).collect()
    }

    /// PAR-29: each pointer the trusted core is told of carries how many
    /// of the picture's pixels a screen pixel covers where the picture is
    /// drawn, which is the span of its rectangle.
    #[test]
    fn the_span_is_the_picture_pixels_a_screen_pixel_covers() {
        for contain in [true, false] {
            let (mut browser, _, picture) = open_picture(contain);
            browser.pointer_moved(picture.x0 + 0.25, picture.y0 + 0.25, 0);
            let spans: Vec<u8> = told_with_spans(&mut browser).iter().map(|pointer| pointer.span).collect();
            assert_eq!(spans, [span_of(PictureLanguage::Regis, picture)], "contain {contain}");
        }
    }

    /// REQ-6377: in each fit, the corners of the rectangle the picture is
    /// drawn in are the picture's corners, and a point of the page outside
    /// it is on no pixel.
    #[test]
    fn the_pointer_is_reported_as_a_pixel_of_the_picture_in_each_fit() {
        for contain in [true, false] {
            let (mut browser, key, picture) = open_picture(contain);
            browser.pointer_moved(picture.x0 + 0.25, picture.y0 + 0.25, 0);
            assert_eq!(told(&mut browser), [RegisPointer { key, at: Some((0, 0)), pressed: false, finger: false, span: 1 }], "contain {contain}");
            browser.pointer_moved(picture.x1 - 0.25, picture.y1 - 0.25, 0);
            assert_eq!(told(&mut browser), [RegisPointer { key, at: Some((799, 479)), pressed: false, finger: false, span: 1 }], "contain {contain}");
            // The same pixel again says nothing.
            browser.pointer_moved(picture.x1 - 0.2, picture.y1 - 0.2, 0);
            assert_eq!(told(&mut browser), []);
            browser.pointer_moved(picture.x0 - 30.0, picture.y0 + 5.0, 0);
            assert_eq!(told(&mut browser), [RegisPointer { key, at: None, pressed: false, finger: false, span: 1 }], "off the picture, contain {contain}");
        }
        // Contained to the teletext page, the band above the picture is the
        // page's and not the picture's.
        let (mut browser, key, picture) = open_picture(false);
        browser.pointer_moved(picture.x0 + 5.0, picture.y0 + 5.0, 0);
        let _ = told(&mut browser);
        browser.pointer_moved(picture.x0 + 5.0, picture.y0 - 5.0, 0);
        assert_eq!(told(&mut browser), [RegisPointer { key, at: None, pressed: false, finger: false, span: 1 }]);
    }

    /// REQ-6374 item 4: the main button going down and coming up on the
    /// picture is told, before anything of the press goes to the channel.
    #[test]
    fn a_press_on_the_picture_is_told_and_released() {
        let (mut browser, key, picture) = open_picture(true);
        let middle = middle_of(picture);
        browser.pointer_moved(middle.x, middle.y, 0);
        let at = told(&mut browser)[0].at;
        browser.regis_button(true);
        assert_eq!(told(&mut browser), [RegisPointer { key, at, pressed: true, finger: false, span: 1 }]);
        browser.pointer_released();
        assert_eq!(told(&mut browser), [RegisPointer { key, at, pressed: false, finger: false, span: 1 }]);
    }

    /// REQ-6374 item 2: the pointer is the hand while the trusted core says
    /// a drawing is under it, and only over the picture.
    #[test]
    fn the_pointer_is_the_hand_while_a_drawing_is_under_it() {
        let (mut browser, key, picture) = open_picture(true);
        let middle = middle_of(picture);
        browser.pointer_moved(middle.x, middle.y, 0);
        let before = browser.cursor();
        assert_ne!(before, Cursor::Hand, "no drawing is known to be under it");
        browser.set_regis_over(key, true);
        assert_eq!(browser.cursor(), Cursor::Hand);
        browser.set_regis_over(key, false);
        assert_eq!(browser.cursor(), before);
    }

    /// REQ-5819, DEC-6667: on a page drawn in NAPLPS, in either fit, the
    /// pointer is told as a dot of the teletext raster the picture fills,
    /// the whole page image, so its cell is the teletext page's: no
    /// 80-pixel offset, and the image's corners are the raster's.
    #[test]
    fn the_pointer_on_a_naplps_picture_is_a_dot_of_the_teletext_raster() {
        for contain in [true, false] {
            let (mut browser, mut canvas) = browser();
            let mut settings = crate::browser::fixture::settings();
            settings.regis_contain = contain;
            browser.set_settings(settings);
            browser.toggle_wall(0);
            browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
            let (key, _) = browser.wall().shown().next().unwrap();
            browser.set_regis(key, 0, 1, PictureLanguage::Naplps);
            browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
            let (_, image) = browser.wall.open().unwrap();
            let _ = browser.take_outputs();
            browser.pointer_moved(image.x0 + 0.25, image.y0 + 0.25, 0);
            assert_eq!(told(&mut browser), [RegisPointer { key, at: Some((0, 0)), pressed: false, finger: false, span: 1 }], "contain {contain}");
            browser.pointer_moved(image.x1 - 0.25, image.y1 - 0.25, 0);
            assert_eq!(told(&mut browser), [RegisPointer { key, at: Some((479, 499)), pressed: false, finger: false, span: 1 }], "contain {contain}");
            // The middle of cell (10, 12): column 10's dots are 120 to 131.
            let dot = |across: f64, down: f64| Point::new(image.x0 + across * image.width() / 480.0, image.y0 + down * image.height() / 500.0);
            let middle = dot(10.0 * 12.0 + 6.0, 12.0 * 20.0 + 10.0);
            browser.pointer_moved(middle.x, middle.y, 0);
            assert_eq!(told(&mut browser), [RegisPointer { key, at: Some((126, 250)), pressed: false, finger: false, span: 1 }], "contain {contain}");
        }
    }

    /// A page that is not drawn in ReGIS tells the trusted core nothing.
    #[test]
    fn a_teletext_page_tells_nothing() {
        let (mut browser, mut canvas) = browser();
        browser.toggle_wall(0);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let _ = browser.take_outputs();
        let (_, image) = browser.wall.open().unwrap();
        browser.pointer_moved(middle_of(image).x, middle_of(image).y, 0);
        assert_eq!(told(&mut browser), []);
    }
}
