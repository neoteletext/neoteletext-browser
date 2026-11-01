//! A selection the channel refused (`INPUT.md` section 7.12, REQ-1481):
//! while the strip says so, the page area is cleared and says why in the
//! browser's own words, by the status alone, as a web browser's error page
//! does; a channel writes nothing here. It asks nothing and has no buttons,
//! and the next selection takes it away.

use super::{Browser, Denied};
use crate::canvas::{Canvas, Style};
use crate::geometry::Rect;
use crate::text::centred;
use crate::theme::{BLACK, HEADING, ROW_TITLE, WHITE};

/// The first line's style: a heading's, with its capitals as written.
const HEADLINE_STYLE: Style = Style { caps: false, ..HEADING };
/// Between the first line and the second.
const GAP: f64 = 12.0;

/// What the page area says of a refusal: its first line, and its second.
const fn words(denied: Denied) -> (&'static str, &'static str) {
    match denied {
        Denied::Forbidden => ("403: not allowed", "This channel does not allow you to see this page."),
        Denied::NotFound => ("404: no such page", "This channel has no page here."),
        Denied::TooMany => ("429: too many requests", "This channel has reached a limit. Try again in a moment."),
        Denied::Unavailable => ("503: not answering", "This channel's program is not answering. Try again in a moment."),
    }
}

impl Browser {
    /// Clears `area`, the page area, and says why the channel refused what
    /// the tab selected, while it did.
    pub(super) fn draw_denied<C: Canvas + ?Sized>(&mut self, canvas: &mut C, area: Rect) {
        let Some(denied) = self.strip.denied else { return };
        canvas.fill(area, 0.0, BLACK);
        let (headline, meaning) = words(denied);
        let (first, second) = (canvas.line_height(HEADLINE_STYLE), canvas.line_height(ROW_TITLE));
        let top = f64::midpoint(area.y0, area.y1) - (first + GAP + second) / 2.0;
        centred(canvas, Rect::new(area.x0, top, area.x1, top + first), headline, HEADLINE_STYLE, WHITE);
        let below = top + first + GAP;
        centred(canvas, Rect::new(area.x0, below, area.x1, below + second), meaning, ROW_TITLE, WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::StripState;
    use crate::browser::fixture::{HEIGHT, WIDTH, browser};
    use crate::test_canvas::Recorder;

    fn drawn(browser: &mut Browser) -> Recorder {
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        canvas
    }

    /// `INPUT.md` section 7.12: each of the four statuses has its own
    /// words, on the page area, in place of the pages. The next selection
    /// takes them away. On the code before, the viewer saw a console line.
    #[test]
    fn a_refused_selection_is_said_on_the_page_area_by_its_status() {
        let (mut browser, _) = browser();
        for (status, first, second) in [
            (403, "403: not allowed", "This channel does not allow you to see this page."),
            (404, "404: no such page", "This channel has no page here."),
            (429, "429: too many requests", "This channel has reached a limit. Try again in a moment."),
            (503, "503: not answering", "This channel's program is not answering. Try again in a moment."),
        ] {
            browser.set_strip(StripState { denied: Denied::of_status(status), ..browser.strip.clone() });
            let canvas = drawn(&mut browser);
            let cells = browser.wall.area();
            for text in [first, second] {
                let (rect, _) = canvas.texts.iter().find(|(_, drawn)| drawn == text).unwrap_or_else(|| panic!("{text}"));
                assert!(
                    cells.contains(crate::geometry::Point { x: f64::midpoint(rect.x0, rect.x1), y: f64::midpoint(rect.y0, rect.y1) }),
                    "{text} in {rect:?}"
                );
            }
        }
        browser.set_strip(StripState { denied: None, ..browser.strip.clone() });
        let canvas = drawn(&mut browser);
        assert!(!canvas.texts.iter().any(|(_, text)| text.starts_with("503")), "gone with the refusal");
    }

    /// Each status a DENIED record carries, and no other.
    #[test]
    fn the_status_numbers_are_the_records() {
        for status in [403, 404, 429, 503] {
            assert_eq!(Denied::of_status(status).map(Denied::status), Some(status));
        }
        assert_eq!(Denied::of_status(500), None);
        assert_eq!(Denied::of_status(0), None);
    }
}
