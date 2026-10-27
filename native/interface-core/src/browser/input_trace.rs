//! The input trace drawn (`trace.rs`): over the open page's row 0, the
//! number of input events sent so far and the last one, and, while the
//! same event goes again and again, how many times running, in orange so
//! a flood stands out. Only with the trusted core's `--input-trace`.

use alloc::format;
use alloc::string::String;

use crate::browser::Browser;
use crate::canvas::{Canvas, ROWS, Style};
use crate::geometry::Rect;
use crate::theme::{BLACK, ORANGE, YELLOW};
use crate::trace::TraceLine;

/// The trace's text against row 0's height, which it fills with a little room.
const TEXT_SHARE: f64 = 0.8;
/// Room before the text, against row 0's height.
const INSET_SHARE: f64 = 0.3;

impl Browser {
    /// The trusted core sent an input event: its line replaces the last.
    pub fn set_input_trace(&mut self, line: TraceLine) {
        self.input_trace = Some(line);
    }

    /// The trace over the open page's row 0, when there is one.
    pub(super) fn draw_input_trace<C: Canvas + ?Sized>(&self, canvas: &mut C) {
        let (Some(line), Some((_, page))) = (&self.input_trace, self.wall.open()) else { return };
        let height = page.height() / f64::from(ROWS);
        let row = Rect::new(page.x0, page.y0, page.x1, page.y0 + height);
        canvas.fill(row, 0.0, BLACK);
        let repeats = if line.repeats > 1 { format!("  x{}", line.repeats) } else { String::new() };
        let text = format!("#{} {}{repeats}", line.sent, line.text);
        #[allow(clippy::cast_possible_truncation, reason = "a row's height in logical pixels, far inside f32")]
        let style = Style { size: (height * TEXT_SHARE) as f32, monospace: true, bold: true, caps: false };
        let inset = height * INSET_SHARE;
        canvas.push_clip(row);
        canvas.text(Rect::new(row.x0 + inset, row.y0, row.x1, row.y1), &text, style, if line.repeats > 1 { ORANGE } else { YELLOW });
        canvas.pop_clip();
    }
}

#[cfg(test)]
mod tests {
    use crate::browser::fixture::{HEIGHT, WIDTH, browser, press_at};
    use crate::theme::{ORANGE, YELLOW};
    use crate::trace::TraceLine;

    /// The trace shows on the open page's top row, the count of sends and
    /// the event, and the repeats in orange; with no page open, nothing.
    #[test]
    fn the_trace_is_drawn_on_the_open_pages_row_0() {
        let (mut browser, mut canvas) = browser();
        browser.set_input_trace(TraceLine { text: "MOUSE move b0 12,5".into(), repeats: 3, sent: 41 });
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        assert_eq!(canvas.coloured_text("#41 MOUSE move b0 12,5  x3"), None, "no page open, nothing drawn");
        let (_, rect) = browser.wall().shown().next().unwrap();
        press_at(&mut browser, &mut canvas, rect);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 1_000);
        assert_eq!(canvas.coloured_text("#41 MOUSE move b0 12,5  x3"), Some(ORANGE));
        browser.set_input_trace(TraceLine { text: "PAGE 128".into(), repeats: 1, sent: 42 });
        browser.draw(&mut canvas, WIDTH, HEIGHT, 2_000);
        assert_eq!(canvas.coloured_text("#42 PAGE 128"), Some(YELLOW));
    }
}
