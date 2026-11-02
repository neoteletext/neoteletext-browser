//! A selection the channel refused (DOC-1961, REQ-1481), and a tab that
//! failed (REQ-683): while the strip says so, the page area is cleared and
//! says why in the browser's own words, by the status alone, as a web
//! browser's error page does; a channel writes nothing here. It asks nothing
//! and has no buttons, and the next selection or a reload takes it away.

use super::{Browser, Denied, Failed};
use alloc::borrow::ToOwned as _;
use alloc::format;
use alloc::string::String;

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

/// What the page area says of a tab that failed: its first line, and its
/// second. Nothing a server sent is said but a number.
fn failure_words(failed: Failed) -> (String, &'static str) {
    match failed {
        Failed::Decoder => {
            ("This channel's pages stopped".to_owned(), "The part of the browser that reads them misbehaved and was closed. Reload to try again.")
        }
        Failed::Fetcher => {
            ("This channel's connection stopped".to_owned(), "The part of the browser that fetches it misbehaved and was closed. Reload to try again.")
        }
        Failed::Channel(Some(status)) => (format!("{status}: not a channel"), "This address did not answer with a teletext channel."),
        Failed::Channel(None) => ("Channel not reached".to_owned(), "This address could not be reached. Try again in a moment."),
        Failed::Page(Some(status)) => match Denied::of_status(status) {
            Some(denied) => {
                let (headline, meaning) = words(denied);
                (headline.to_owned(), meaning)
            }
            None => (format!("{status}: not a page"), "This channel did not answer with a teletext page."),
        },
        Failed::Page(None) => ("Page not reached".to_owned(), "This page could not be fetched. Try again in a moment."),
    }
}

impl Browser {
    /// Clears `area`, the page area, and says why the channel refused what
    /// the tab selected, while it did.
    pub(super) fn draw_denied<C: Canvas + ?Sized>(&mut self, canvas: &mut C, area: Rect) {
        let Some(denied) = self.strip.denied else { return };
        let (headline, meaning) = words(denied);
        draw_reason(canvas, area, headline, meaning);
    }

    /// Clears `area`, the page area, and says why the tab failed, while it
    /// has.
    pub(super) fn draw_failed<C: Canvas + ?Sized>(&mut self, canvas: &mut C, area: Rect) {
        let Some(failed) = self.strip.failed else { return };
        let (headline, meaning) = failure_words(failed);
        draw_reason(canvas, area, &headline, meaning);
    }
}

/// Clears `area` and says `headline` and `meaning`, centred.
fn draw_reason<C: Canvas + ?Sized>(canvas: &mut C, area: Rect, headline: &str, meaning: &str) {
    canvas.fill(area, 0.0, BLACK);
    let (first, second) = (canvas.line_height(HEADLINE_STYLE), canvas.line_height(ROW_TITLE));
    let top = f64::midpoint(area.y0, area.y1) - (first + GAP + second) / 2.0;
    centred(canvas, Rect::new(area.x0, top, area.x1, top + first), headline, HEADLINE_STYLE, WHITE);
    let below = top + first + GAP;
    centred(canvas, Rect::new(area.x0, below, area.x1, below + second), meaning, ROW_TITLE, WHITE);
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

    /// DOC-1961: each of the four statuses has its own
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

    /// REQ-683: a tab whose helper broke its protocol says so in the page
    /// area, in the browser's words, and a reload's new channel takes it
    /// away.
    #[test]
    fn a_failed_tab_says_so_on_the_page_area() {
        let (mut browser, _) = browser();
        for failed in [
            Failed::Decoder,
            Failed::Fetcher,
            Failed::Channel(Some(500)),
            Failed::Channel(None),
            Failed::Page(Some(200)),
            Failed::Page(Some(404)),
            Failed::Page(None),
        ] {
            browser.set_strip(StripState { failed: Some(failed), ..browser.strip.clone() });
            let canvas = drawn(&mut browser);
            let (first, second) = failure_words(failed);
            let cells = browser.wall.area();
            for text in [first.as_str(), second] {
                let (rect, _) = canvas.texts.iter().find(|(_, drawn)| drawn == text).unwrap_or_else(|| panic!("{text}"));
                assert!(
                    cells.contains(crate::geometry::Point { x: f64::midpoint(rect.x0, rect.x1), y: f64::midpoint(rect.y0, rect.y1) }),
                    "{text} in {rect:?}"
                );
            }
        }
        browser.set_strip(StripState { failed: None, ..browser.strip.clone() });
        assert!(!drawn(&mut browser).texts.iter().any(|(_, text)| text.contains("Page not reached")), "gone with the failure");
    }

    /// REQ-708: the first line states the status; a status that a DENIED
    /// record carries reads as that record does, so a static channel's
    /// missing page reads as a live channel's.
    #[test]
    fn a_refusal_states_its_status() {
        assert_eq!(failure_words(Failed::Page(Some(200))).0, "200: not a page");
        assert_eq!(failure_words(Failed::Page(Some(500))).0, "500: not a page");
        assert_eq!(failure_words(Failed::Page(Some(404))).0, words(Denied::NotFound).0);
        assert_eq!(failure_words(Failed::Channel(Some(403))).0, "403: not a channel");
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
