//! The line round a window the browser frames itself (REQ-6396): the
//! system draws no border there, so the window's edge, which is where a
//! press resizes it, would otherwise not show against what lies behind.

use crate::browser::Browser;
use crate::canvas::Canvas;
use crate::geometry::Rect;
use crate::theme::RULE;

/// The line's thickness, in logical pixels: the least that shows.
const BORDER: f64 = 1.0;

impl Browser {
    /// Whether the window fills its display, as the host says before each
    /// frame: a maximised window has no edge to show or to drag.
    pub const fn set_window_maximised(&mut self, maximised: bool) {
        self.window_maximised = maximised;
    }

    /// Whether the window's border is drawn: where the browser frames the
    /// window, while it is neither maximised nor full screen.
    #[must_use]
    pub fn window_bordered(&self) -> bool {
        self.frame.window_buttons && !self.window_maximised && !self.full_screen()
    }

    /// Draws the border over the window's edges, last of all.
    pub fn draw_window_border<C: Canvas + ?Sized>(&self, canvas: &mut C, width: f64, height: f64) {
        if !self.window_bordered() {
            return;
        }
        for edge in [
            Rect::new(0.0, 0.0, width, BORDER),
            Rect::new(0.0, height - BORDER, width, height),
            Rect::new(0.0, 0.0, BORDER, height),
            Rect::new(width - BORDER, 0.0, width, height),
        ] {
            canvas.fill(edge, 0.0, RULE);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::browser::fixture::{HEIGHT, WIDTH, browser};
    use crate::browser::{Browser, Frame};
    use crate::geometry::Rect;
    use crate::test_canvas::Recorder;
    use crate::theme::RULE;

    fn edges(browser: &Browser) -> alloc::vec::Vec<Rect> {
        let mut canvas = Recorder::new();
        browser.draw_window_border(&mut canvas, WIDTH, HEIGHT);
        canvas.fills.iter().filter(|(_, colour)| *colour == RULE).map(|(rect, _)| *rect).collect()
    }

    /// REQ-6396: a window the browser frames has a line along each of its
    /// four edges; a maximised one, a full-screen one and one the system
    /// frames have none.
    #[test]
    fn a_window_the_browser_frames_has_a_line_round_it_unless_it_fills_the_display() {
        let (mut browser, mut canvas) = browser();
        assert_eq!(
            edges(&browser),
            [
                Rect::new(0.0, 0.0, WIDTH, 1.0),
                Rect::new(0.0, HEIGHT - 1.0, WIDTH, HEIGHT),
                Rect::new(0.0, 0.0, 1.0, HEIGHT),
                Rect::new(WIDTH - 1.0, 0.0, WIDTH, HEIGHT)
            ]
        );
        browser.set_window_maximised(true);
        assert_eq!(edges(&browser), [], "maximised");
        browser.set_window_maximised(false);
        browser.toggle_wall(0);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        assert!(browser.toggle_full_screen(0), "a page is open to fill the screen");
        assert_eq!(edges(&browser), [], "full screen");
        let framed_by_the_system = Browser::new(Frame { left_inset: 72.0, window_buttons: false });
        assert_eq!(edges(&framed_by_the_system), [], "the system's frame");
    }
}
