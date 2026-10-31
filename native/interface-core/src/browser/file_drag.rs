//! Files dragged over the window (DEC-1549 items 3 and 5, as Chrome does):
//! an arrow marks where they would land on the tab strip, and resting over
//! the middle of a background tab brings it forward. Only where the system
//! says where the pointer is during a drag does any of this run.

use crate::browser::toolbar::TOOLBAR_TOP;
use crate::browser::{Browser, DropPlace, DropTarget, Output};
use crate::canvas::Canvas;
use crate::geometry::{Point, Rect};
use crate::theme::LINK;

/// The arrow's height and width: Chrome's 20 pixels.
const ARROW: f64 = 20.0;
/// How long the pointer rests on a background tab before it comes forward:
/// Chrome's `hover_tab_selector.cc`.
const HOVER_TO_SWITCH: u64 = 500;

/// A drag of files over the window.
#[derive(Debug, Default)]
pub(super) struct FileDrag {
    /// Files are over the window, whether or not the pointer can be told.
    under_way: bool,
    /// Where the pointer is.
    at: Option<Point>,
    /// The background tab it rests on, and since when.
    resting: Option<(usize, u64)>,
}

impl Browser {
    /// Files have come over the window.
    pub fn file_drag_began(&mut self) {
        self.file_drag.under_way = true;
    }

    /// Whether files are over the window.
    #[must_use]
    pub fn files_dragged(&self) -> bool {
        self.file_drag.under_way
    }

    /// Files are dragged over `at`, or no longer are. Resting on the middle
    /// of a background tab brings it forward.
    pub fn file_drag(&mut self, at: Option<Point>, now: u64) {
        self.file_drag.under_way = at.is_some();
        self.file_drag.at = at;
        let over = at.map(|at| self.drop_target(at)).and_then(|target| match target {
            DropTarget::Tabs(DropPlace::Replace(index)) => Some(usize::from(index)).filter(|&index| index != self.active),
            _ => None,
        });
        self.file_drag.resting = match (over, self.file_drag.resting) {
            (Some(tab), Some((rested, since))) if tab == rested => {
                if now.saturating_sub(since) >= HOVER_TO_SWITCH {
                    self.outputs.push(Output::SelectTab(tab));
                    None
                } else {
                    Some((rested, since))
                }
            }
            (Some(tab), _) => Some((tab, now)),
            (None, _) => None,
        };
    }

    /// Where on the tab strip dragged files would land, as a place along
    /// it: at a tab's edge, or in its middle when they would replace it.
    fn drop_edge(&self, at: Point) -> Option<f64> {
        let on_strip = at.y < TOOLBAR_TOP;
        let tab = |index: u8| self.regions.tabs.iter().find(|tab| tab.index == usize::from(index)).map(|tab| tab.tab);
        match self.drop_target(at) {
            DropTarget::Tabs(DropPlace::Before(index)) => tab(index).map(|tab| tab.x0),
            DropTarget::Tabs(DropPlace::After(index)) => tab(index).map(|tab| tab.x1),
            DropTarget::Tabs(DropPlace::Replace(index)) if on_strip => tab(index).map(|tab| f64::midpoint(tab.x0, tab.x1)),
            DropTarget::Tabs(DropPlace::NewTabs) if on_strip => self.regions.tabs.last().map(|tab| tab.tab.x1),
            _ => None,
        }
    }

    /// The arrow: above the strip pointing down, or below it pointing up
    /// when there is no room above, as the strip is at the window's top.
    pub(super) fn draw_drop_arrow<C: Canvas + ?Sized>(&self, canvas: &mut C) {
        let Some(x) = self.file_drag.at.and_then(|at| self.drop_edge(at)) else { return };
        let Some(strip) = self.regions.tabs.first().map(|tab| tab.tab) else { return };
        canvas.polygon(&arrow(x, strip), LINK);
    }
}

/// The arrow's corners at `x` on a strip at `strip`, its point at the strip.
fn arrow(x: f64, strip: Rect) -> [Point; 3] {
    let half = ARROW / 2.0;
    if strip.y0 >= ARROW {
        [Point::new(x, strip.y0), Point::new(x - half, strip.y0 - ARROW), Point::new(x + half, strip.y0 - ARROW)]
    } else {
        [Point::new(x, strip.y1), Point::new(x - half, strip.y1 + ARROW), Point::new(x + half, strip.y1 + ARROW)]
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;
    use crate::browser::fixture::{HEIGHT, WIDTH, browser};
    use crate::test_canvas::Recorder;

    // DEC-1549 item 3: 500 milliseconds over the middle of a background tab
    // brings it forward, moving off starts again, and the active tab has
    // nothing to bring forward.
    #[test]
    fn resting_on_a_background_tab_brings_it_forward() {
        let (mut browser, _) = browser();
        let other = browser.regions.tabs.iter().find(|tab| tab.index != browser.active).unwrap().tab;
        let middle = Point::new(f64::midpoint(other.x0, other.x1), f64::midpoint(other.y0, other.y1));
        browser.file_drag(Some(middle), 1_000);
        browser.file_drag(Some(middle), 1_000 + HOVER_TO_SWITCH - 1);
        assert!(!browser.take_outputs().iter().any(|output| matches!(output, Output::SelectTab(_))), "too soon");
        browser.file_drag(Some(Point::new(1.0, 500.0)), 1_000 + HOVER_TO_SWITCH - 1);
        browser.file_drag(Some(middle), 1_000 + HOVER_TO_SWITCH);
        assert!(!browser.take_outputs().iter().any(|output| matches!(output, Output::SelectTab(_))), "it moved off");
        browser.file_drag(Some(middle), 1_000 + 2 * HOVER_TO_SWITCH);
        let index = browser.regions.tabs.iter().find(|tab| tab.index != browser.active).unwrap().index;
        assert!(browser.take_outputs().iter().any(|output| matches!(output, Output::SelectTab(tab) if *tab == index)));
    }

    // DEC-1549 item 3: the arrow marks a tab's edge, or its middle when the
    // tab would be replaced, and shows only over the strip, with no room
    // above it here; DEC-1549 item 5: the page has no mark.
    #[test]
    fn the_arrow_marks_where_files_would_land() {
        let (mut browser, _) = browser();
        let first = browser.regions.tabs.first().unwrap().tab;
        let along = |fraction: f64| Point::new(first.x0 + first.width() * fraction, f64::midpoint(first.y0, first.y1));
        assert_eq!(browser.drop_edge(along(0.1)), Some(first.x0));
        assert_eq!(browser.drop_edge(along(0.5)), Some(f64::midpoint(first.x0, first.x1)));
        assert_eq!(browser.drop_edge(along(0.9)), Some(first.x1));
        assert_eq!(browser.drop_edge(Point::new(1.0, 500.0)), None, "the page has no mark");
        let [point, ..] = arrow(100.0, first);
        assert_eq!(point.y, first.y1, "no room above: below the strip, pointing up");
        let [point, left, right] = arrow(100.0, Rect::new(0.0, 40.0, 100.0, 60.0));
        assert_eq!((point.y, left.y, right.x - left.x), (40.0, 20.0, ARROW), "above it, pointing down, 20 pixels");
        browser.file_drag(Some(along(0.5)), 0);
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        assert!(canvas.polygons.iter().any(|(points, colour)| *colour == LINK && points.len() == 3), "drawn");
    }
}
