//! The pointer on the open page's ReGIS picture (REQ-6377, REQ-6374): the
//! trusted core is told the pixel of the picture under it and whether it
//! is pressed, and says back whether a drawing a click would act on lies
//! there, which makes the pointer the hand. What is drawn where is the
//! trusted core's knowledge alone.

use super::Browser;
use super::output::Output;
use crate::canvas::{PageKey, regis_contained, regis_within};
use crate::geometry::Point;
use crate::regis::{RegisPointer, pixel_under};

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
    /// The open page, if it is drawn in ReGIS, and the pixel of its
    /// picture under `point`, if any: the picture is drawn in the page's
    /// image as the painter places it, by the fit in force (REQ-6363).
    fn regis_pixel(&self, point: Point) -> Option<(PageKey, Option<(u16, u16)>)> {
        let (key, image) = self.wall.open()?;
        if !self.regis.contains_key(&key) {
            return None;
        }
        let picture = if self.regis_contained() { regis_contained(image) } else { regis_within(image) };
        Some((key, pixel_under(picture, point)))
    }

    /// Tells the trusted core where the pointer is on the open page's
    /// picture, when that or its press has changed since it was last told;
    /// and that it has left, when the page is no longer open or drawn in
    /// ReGIS. Called before any input of the same moment is sent to the
    /// channel, so that what the trusted core looks up for that input is
    /// of this place.
    pub(super) fn regis_pointed(&mut self) {
        let finger = self.regis_pointer.finger;
        let wanted = self.regis_pixel(finger.unwrap_or(self.pointer)).map(|(key, at)| RegisPointer {
            key,
            at,
            pressed: at.is_some() && (self.regis_pointer.pressed || finger.is_some()),
            finger: finger.is_some(),
        });
        let sent = self.regis_pointer.sent;
        if wanted == sent {
            return;
        }
        // Gone from a page it was on: that page's pointer is off its picture.
        if let Some(sent) = sent.filter(|sent| wanted.is_none_or(|wanted| wanted.key != sent.key) && sent.at.is_some()) {
            self.outputs.push(Output::RegisPointer(RegisPointer { key: sent.key, at: None, pressed: false, finger: false }));
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
        self.regis_pointer.pressed = down && self.regis_pixel(self.pointer).is_some_and(|(_, at)| at.is_some());
        self.regis_pointed();
    }

    /// A finger came down or moved (`Some`), or lifted (`None`).
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
        over.is_some() && self.regis_pixel(self.pointer).is_some_and(|(key, at)| Some(key) == over && at.is_some())
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
        browser.set_regis(key, 0, 1);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let (_, image) = browser.wall.open().unwrap();
        let picture = if contain { regis_contained(image) } else { regis_within(image) };
        let _ = browser.take_outputs();
        (browser, key, picture)
    }

    /// The middle of a rectangle.
    fn middle_of(rect: Rect) -> Point {
        Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1))
    }

    /// The pointers the trusted core was told of since they were last taken.
    fn told(browser: &mut Browser) -> Vec<RegisPointer> {
        browser.take_outputs().into_iter().filter_map(|output| if let Output::RegisPointer(pointer) = output { Some(pointer) } else { None }).collect()
    }

    /// REQ-6377: in each fit, the corners of the rectangle the picture is
    /// drawn in are the picture's corners, and a point of the page outside
    /// it is on no pixel.
    #[test]
    fn the_pointer_is_reported_as_a_pixel_of_the_picture_in_each_fit() {
        for contain in [true, false] {
            let (mut browser, key, picture) = open_picture(contain);
            browser.pointer_moved(picture.x0 + 0.25, picture.y0 + 0.25, 0);
            assert_eq!(told(&mut browser), [RegisPointer { key, at: Some((0, 0)), pressed: false, finger: false }], "contain {contain}");
            browser.pointer_moved(picture.x1 - 0.25, picture.y1 - 0.25, 0);
            assert_eq!(told(&mut browser), [RegisPointer { key, at: Some((799, 479)), pressed: false, finger: false }], "contain {contain}");
            // The same pixel again says nothing.
            browser.pointer_moved(picture.x1 - 0.2, picture.y1 - 0.2, 0);
            assert_eq!(told(&mut browser), []);
            browser.pointer_moved(picture.x0 - 30.0, picture.y0 + 5.0, 0);
            assert_eq!(told(&mut browser), [RegisPointer { key, at: None, pressed: false, finger: false }], "off the picture, contain {contain}");
        }
        // Contained to the teletext page, the band above the picture is the
        // page's and not the picture's.
        let (mut browser, key, picture) = open_picture(false);
        browser.pointer_moved(picture.x0 + 5.0, picture.y0 + 5.0, 0);
        let _ = told(&mut browser);
        browser.pointer_moved(picture.x0 + 5.0, picture.y0 - 5.0, 0);
        assert_eq!(told(&mut browser), [RegisPointer { key, at: None, pressed: false, finger: false }]);
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
        assert_eq!(told(&mut browser), [RegisPointer { key, at, pressed: true, finger: false }]);
        browser.pointer_released();
        assert_eq!(told(&mut browser), [RegisPointer { key, at, pressed: false, finger: false }]);
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
