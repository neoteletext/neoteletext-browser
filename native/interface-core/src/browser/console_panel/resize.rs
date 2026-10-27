//! The console's grabber: a handle in the gap above the console, pulled
//! up or down with the main button to make the console taller or shorter,
//! as a web browser's docked developer tools are. The console is never
//! shorter than its tab bar and a few lines, and never so tall that the
//! pages above it lose their room.

use crate::browser::Browser;
use crate::browser::window::MARGIN;
use crate::canvas::Canvas;
use crate::geometry::{Point, Rect};
use crate::theme::{DIM, MUTED};

use super::{BAR_HEIGHT, GAP, LINE};

/// The shortest console: its tab bar and four lines.
const MIN_HEIGHT: f64 = GAP + BAR_HEIGHT + GAP + 4.0 * LINE + GAP;
/// What the console always leaves the pages above it: the page-size
/// control and a page at a readable size.
const MIN_PAGES_HEIGHT: f64 = 240.0;
/// How far either side of the gap the grabber can be gripped: the gap is
/// one margin high, too thin to find by itself.
const GRIP_REACH: f64 = 4.0;
/// The handle drawn in the gap, so it can be seen and found.
const HANDLE_WIDTH: f64 = 48.0;
const HANDLE_HEIGHT: f64 = 3.0;

impl Browser {
    /// The console's height in a window whose body starts at `top` and
    /// whose console ends at `bottom`: as pulled, within its bounds.
    pub(in crate::browser) fn console_height(&self, top: f64, bottom: f64) -> f64 {
        let tallest = (bottom - top - MIN_PAGES_HEIGHT).max(MIN_HEIGHT);
        self.console.height.clamp(MIN_HEIGHT, tallest)
    }

    /// The grabber's grip over the gap above the console at `area`, and its
    /// handle, lit while the pointer is on it or pulling it.
    pub(in crate::browser) fn draw_console_grabber<C: Canvas + ?Sized>(&mut self, canvas: &mut C, area: Rect) {
        self.regions.console_grabber = Rect::new(area.x0, area.y0 - MARGIN - GRIP_REACH, area.x1, area.y0 + GRIP_REACH);
        let middle = f64::midpoint(area.x0, area.x1);
        let y = area.y0 - MARGIN / 2.0;
        let handle = Rect::new(middle - HANDLE_WIDTH / 2.0, y - HANDLE_HEIGHT / 2.0, middle + HANDLE_WIDTH / 2.0, y + HANDLE_HEIGHT / 2.0);
        canvas.fill(handle, HANDLE_HEIGHT / 2.0, if self.over_console_grabber() { MUTED } else { DIM });
    }

    /// A press on the grabber grips the console's top edge where it was pressed.
    pub(in crate::browser) fn console_grabber_press(&mut self, point: Point) -> bool {
        if !self.console.open || !self.regions.console_grabber.contains(point) {
            return false;
        }
        self.console.grip = Some(point.y - self.console.area.y0);
        true
    }

    /// The pointer moved while the grabber is gripped: the console's top
    /// edge follows it. Returns whether the window needs drawing.
    pub(in crate::browser) fn console_grabber_moved(&mut self, point: Point) -> bool {
        let Some(grip) = self.console.grip else { return false };
        self.console.height = self.console.area.y1 - (point.y - grip);
        true
    }

    /// The main button let go: the console keeps the height it was left at,
    /// within the bounds the window gives it.
    pub(in crate::browser) fn console_grabber_released(&mut self) {
        if self.console.grip.take().is_some() {
            self.console.height = self.console_height(self.body.y0, self.console.area.y1);
        }
    }

    /// Whether the pointer is over the grabber or pulling it.
    pub(in crate::browser) fn over_console_grabber(&self) -> bool {
        self.console.open && (self.console.grip.is_some() || self.regions.console_grabber.contains(self.pointer))
    }
}
