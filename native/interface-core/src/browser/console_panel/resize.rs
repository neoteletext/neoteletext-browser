//! The console's grabber: a handle in the gap above the console, pulled
//! up or down with the main button to make the console taller or shorter,
//! as a web browser's docked developer tools are. The console is never
//! shorter than its tab bar and a few lines, and never so tall that the
//! pages above it lose their room. Its minimise button folds it to its
//! tab bar, as the overview folds to its heading, and its maximise button
//! gives it that tallest height; pressed again, each restores the height
//! the grabber left, and the grabber pulled leaves either (decision 120).

use crate::browser::Browser;
use crate::browser::window::MARGIN;
use crate::canvas::Canvas;
use crate::geometry::{Point, Rect};
use crate::theme::{DIM, MUTED};

use super::{BAR_HEIGHT, ConsoleUi, GAP, LINE, Sizing};

/// The shortest console the grabber leaves: its tab bar and four lines.
pub(super) const MIN_HEIGHT: f64 = GAP + BAR_HEIGHT + GAP + 4.0 * LINE + GAP;
/// The console minimised: its tab bar between its gaps.
pub(super) const MINIMISED_HEIGHT: f64 = GAP + BAR_HEIGHT + GAP;
/// What the console always leaves the pages above it: the page-size
/// control and a page at a readable size.
pub(super) const MIN_PAGES_HEIGHT: f64 = 240.0;
/// How far either side of the gap the grabber can be gripped: the gap is
/// one margin high, too thin to find by itself.
const GRIP_REACH: f64 = 4.0;
/// The handle drawn in the gap, so it can be seen and found.
const HANDLE_WIDTH: f64 = 48.0;
const HANDLE_HEIGHT: f64 = 3.0;

impl Browser {
    /// The console's height in a window whose body starts at `top` and
    /// whose console ends at `bottom`: its tab bar's when minimised, the
    /// tallest the pages allow when maximised, and otherwise as pulled,
    /// within its bounds.
    pub(in crate::browser) fn console_height(&self, top: f64, bottom: f64) -> f64 {
        match self.console.sizing {
            Sizing::Minimised => MINIMISED_HEIGHT,
            Sizing::Maximised => tallest(top, bottom),
            Sizing::Pulled => self.console.height.clamp(MIN_HEIGHT, tallest(top, bottom)),
        }
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
    /// Pulled from minimised or maximised, it leaves that state from the
    /// height it had.
    pub(in crate::browser) fn console_grabber_moved(&mut self, point: Point) -> bool {
        let Some(grip) = self.console.grip else { return false };
        self.console.sizing = Sizing::Pulled;
        self.console.height = self.console.area.y1 - (point.y - grip);
        true
    }

    /// The main button let go: the console keeps the height it was left at,
    /// within the bounds the window gives it. A press that never moved
    /// leaves a minimised or maximised console as it was.
    pub(in crate::browser) fn console_grabber_released(&mut self) {
        if self.console.grip.take().is_some() && self.console.sizing == Sizing::Pulled {
            self.console.height = self.console_height(self.body.y0, self.console.area.y1);
        }
    }

    /// Whether the pointer is over the grabber or pulling it.
    pub(in crate::browser) fn over_console_grabber(&self) -> bool {
        self.console.open && (self.console.grip.is_some() || self.regions.console_grabber.contains(self.pointer))
    }
}

/// The tallest console in a window whose body starts at `top` and whose
/// console ends at `bottom`: what the pages above it can spare.
fn tallest(top: f64, bottom: f64) -> f64 {
    (bottom - top - MIN_PAGES_HEIGHT).max(MIN_HEIGHT)
}

impl ConsoleUi {
    /// Its minimise button: folded to its tab bar, or restored.
    pub(super) fn toggle_minimised(&mut self) {
        self.sizing = if self.sizing == Sizing::Minimised { Sizing::Pulled } else { Sizing::Minimised };
    }

    /// Its maximise button: as tall as the pages allow, or restored.
    pub(super) fn toggle_maximised(&mut self) {
        self.sizing = if self.sizing == Sizing::Maximised { Sizing::Pulled } else { Sizing::Maximised };
    }

    /// Whether it is folded to its tab bar.
    pub(super) fn minimised(&self) -> bool {
        self.sizing == Sizing::Minimised
    }

    /// Whether it is as tall as the pages allow.
    pub(super) fn maximised(&self) -> bool {
        self.sizing == Sizing::Maximised
    }

    /// Whether `point` is on its minimise, maximise or close button.
    pub(in crate::browser) fn over_window_button(&self, point: Point) -> bool {
        self.open && [self.minimise, self.maximise, self.close].iter().any(|button| button.contains(point))
    }
}
