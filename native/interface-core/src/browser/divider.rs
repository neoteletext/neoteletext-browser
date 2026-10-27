//! The divider between the page list and the page area: pulled with the
//! main button, it moves the boundary between them, the list never
//! narrower than its titles need and the page area never narrower than a
//! page at its smallest.

use crate::browser::Browser;
use crate::browser::window::MARGIN;
use crate::canvas::PAGE_WIDTH;
use crate::geometry::{Point, Rect};

/// The page list's width until the divider is pulled.
pub(super) const LIST_WIDTH: f64 = 260.0;
/// The narrowest list: a page number and a few words of its title.
const MIN_LIST_WIDTH: f64 = 160.0;
/// The narrowest page area: a page at the engine's own size, a pixel a
/// dot, and the page-size control below it.
const MIN_PAGE_AREA_WIDTH: f64 = PAGE_WIDTH as f64;
/// How far either side of the gap the divider can be gripped: the gap is
/// one margin wide, too thin to find by itself.
const GRIP_REACH: f64 = 4.0;

impl Browser {
    /// The list's width in a body whose list starts at `left` and whose
    /// page area ends at `right`: as pulled, within its bounds.
    pub(super) fn list_width(&self, left: f64, right: f64) -> f64 {
        let widest = right - left - MARGIN - MIN_PAGE_AREA_WIDTH;
        self.list_width.min(widest).max(MIN_LIST_WIDTH)
    }

    /// The divider's grip, the gap between `list` and the page area and a
    /// little either side.
    pub(super) fn place_divider(&mut self, list: Rect) {
        self.regions.divider = Rect::new(list.x1 - GRIP_REACH, list.y0, list.x1 + MARGIN + GRIP_REACH, list.y1);
    }

    /// A press on the divider grips it where it was pressed.
    pub(super) fn divider_press(&mut self, point: Point) -> bool {
        if !self.regions.divider.contains(point) {
            return false;
        }
        self.divider_grip = Some(point.x - self.regions.list.x1);
        true
    }

    /// The pointer moved while the divider is gripped: the list follows it.
    /// Returns whether the window needs drawing.
    pub(super) fn divider_moved(&mut self, point: Point) -> bool {
        let Some(grip) = self.divider_grip else { return false };
        self.list_width = point.x - grip - self.regions.list.x0;
        true
    }

    /// The main button let go: the divider stays where it was left, within
    /// the bounds the window gives it.
    pub(super) fn divider_released(&mut self) {
        if self.divider_grip.take().is_some() {
            self.list_width = self.list_width(self.regions.list.x0, self.body.x1);
        }
    }

    /// Whether the pointer is over the divider or pulling it.
    pub(super) fn over_divider(&self) -> bool {
        self.divider_grip.is_some() || self.regions.divider.contains(self.pointer)
    }
}

#[cfg(test)]
mod tests {
    use crate::browser::Browser;
    use crate::browser::fixture::{HEIGHT, WIDTH, browser};
    use crate::browser::pointer::Cursor;
    use crate::test_canvas::Recorder;

    fn redraw(browser: &mut Browser, canvas: &mut Recorder) {
        browser.draw(canvas, WIDTH, HEIGHT, 0);
    }

    #[test]
    fn the_divider_pulls_the_list_wider_and_narrower_within_its_bounds() {
        let (mut browser, mut canvas) = browser();
        let list = browser.regions.list;
        let gap = list.x1 + super::MARGIN / 2.0;
        let middle = f64::midpoint(list.y0, list.y1);
        browser.pointer_moved(gap, middle, 0);
        assert_eq!(browser.cursor(), Cursor::ResizeColumn);
        browser.pointer_pressed(&mut canvas, false, 0);
        assert!(browser.pointer_moved(gap + 100.0, middle, 0));
        redraw(&mut browser, &mut canvas);
        assert!((browser.regions.list.width() - (list.width() + 100.0)).abs() < 1e-9, "the list follows the pointer");
        browser.pointer_moved(0.0, middle, 0);
        redraw(&mut browser, &mut canvas);
        assert!((browser.regions.list.width() - super::MIN_LIST_WIDTH).abs() < 1e-9, "no narrower than its titles need");
        browser.pointer_moved(WIDTH, middle, 0);
        browser.pointer_released();
        redraw(&mut browser, &mut canvas);
        let area = browser.body.x1 - browser.regions.list.x1 - super::MARGIN;
        assert!((area - super::MIN_PAGE_AREA_WIDTH).abs() < 1e-9, "the page area keeps a page's width: {area}");
        browser.pointer_moved(gap, middle, 0);
        redraw(&mut browser, &mut canvas);
        assert_ne!(browser.cursor(), Cursor::ResizeColumn, "let go, the divider stays where it was left, away from the pointer");
    }
}
