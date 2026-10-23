//! The certificate-error screen (`certificate-errors.md`, decision 83):
//! while the strip says the channel's certificate was refused, the page
//! area shows why, in the trusted core's words, and offers Try again, Go
//! back and Close tab; nothing opens the channel anyway (decision 24).

use alloc::string::String;
use alloc::vec::Vec;

use super::widgets::{button, panel};
use super::{Browser, Output};
use crate::canvas::{Canvas, Style};
use crate::geometry::{Point, Rect};
use crate::text::left_centred;
use crate::theme::{DIM, HEADING, INSECURE, ROW_STATUS, ROW_TITLE, WHITE};

/// The screen's panel, at most this wide; narrower areas take it whole.
const PANEL_WIDTH: f64 = 640.0;
const PADDING: f64 = 28.0;
const GAP: f64 = 12.0;
const LINE_GAP: f64 = 4.0;
const BUTTON_WIDTH: f64 = 132.0;
const BUTTON_HEIGHT: f64 = 32.0;
/// What the screen says of every refusal (`certificate-errors.md` section 2).
const HEADING_TEXT: &str = "This channel's certificate was refused";
const MEANING: &str = "NeoTeletext does not open a channel whose certificate fails. There is no way past this screen.";

/// Where the screen's buttons were last drawn; empty while it does not show.
#[derive(Debug, Default)]
pub(super) struct RefusedUi {
    try_again: Rect,
    back: Rect,
    close: Rect,
}

/// `text` broken into lines no wider than `width` in `style`, at spaces.
fn wrapped<C: Canvas + ?Sized>(canvas: &mut C, text: &str, style: Style, width: f64) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let candidate = if line.is_empty() { String::from(word) } else { alloc::format!("{line} {word}") };
        if !line.is_empty() && canvas.text_width(&candidate, style) > width {
            lines.push(core::mem::take(&mut line));
            line = String::from(word);
        } else {
            line = candidate;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

impl Browser {
    /// Whether the certificate-error screen shows.
    #[must_use]
    pub fn certificate_refused(&self) -> bool {
        self.strip.refused.is_some()
    }

    /// The screen over `area`, the page area, while the channel is refused.
    pub(super) fn draw_refusal<C: Canvas + ?Sized>(&mut self, canvas: &mut C, area: Rect) {
        self.refused = RefusedUi::default();
        let Some(problem) = self.strip.refused else { return };
        canvas.fill(area, 0.0, crate::theme::BLACK);
        let width = PANEL_WIDTH.min(area.width() - 2.0 * GAP);
        let text_width = width - 2.0 * PADDING;
        // Measured first, so the panel is as tall as what it says.
        let line_height = |canvas: &mut C, style: Style| canvas.line_height(style) + LINE_GAP;
        let mut blocks: Vec<(Vec<String>, Style, crate::theme::Colour)> = alloc::vec![
            (wrapped(canvas, HEADING_TEXT, HEADING, text_width), HEADING, INSECURE),
            (wrapped(canvas, &self.strip.identity, ROW_TITLE, text_width), ROW_TITLE, WHITE),
            (wrapped(canvas, problem.sentence(), ROW_TITLE, text_width), ROW_TITLE, WHITE),
            (wrapped(canvas, MEANING, ROW_STATUS, text_width), ROW_STATUS, DIM),
        ];
        if let Some(hint) = problem.hint() {
            blocks.push((wrapped(canvas, hint, ROW_STATUS, text_width), ROW_STATUS, DIM));
        }
        let mut height = 2.0 * PADDING + BUTTON_HEIGHT;
        for (lines, style, _) in &blocks {
            #[allow(clippy::cast_precision_loss, reason = "a few lines")]
            let count = lines.len() as f64;
            height += count * line_height(canvas, *style) + GAP;
        }
        let left = f64::midpoint(area.x0, area.x1) - width / 2.0;
        let top = (f64::midpoint(area.y0, area.y1) - height / 2.0).max(area.y0 + GAP);
        let rect = Rect::new(left, top, left + width, top + height);
        panel(canvas, rect);
        let mut y = top + PADDING;
        for (lines, style, colour) in &blocks {
            let step = line_height(canvas, *style);
            for line in lines {
                left_centred(canvas, Rect::new(left + PADDING, y, left + width - PADDING, y + step), 0.0, line, *style, *colour);
                y += step;
            }
            y += GAP;
        }
        let mut x = left + PADDING;
        let mut place = |word: &str, canvas: &mut C| {
            let at = Rect::new(x, y, x + BUTTON_WIDTH, y + BUTTON_HEIGHT);
            button(canvas, at, word, false);
            x = at.x1 + GAP;
            at
        };
        self.refused.try_again = place("Try again", canvas);
        if self.strip.back {
            self.refused.back = place("Go back", canvas);
        }
        self.refused.close = place("Close tab", canvas);
    }

    /// A press on the screen's buttons; returns whether the screen took it.
    pub(super) fn refused_press(&mut self, point: Point) -> bool {
        if self.strip.refused.is_none() {
            return false;
        }
        let output = if self.refused.try_again.contains(point) {
            Output::Reload
        } else if self.refused.back.contains(point) {
            Output::Back
        } else if self.refused.close.contains(point) {
            Output::CloseTab(self.active)
        } else {
            return false;
        };
        self.outputs.push(output);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::fixture::middle;
    use crate::browser::{CertificateProblem, Frame, StripState};
    use crate::test_canvas::Recorder;

    fn refused(problem: CertificateProblem, back: bool) -> (Browser, Recorder) {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
        browser.set_strip(StripState {
            address: "https://news.example/".into(),
            identity: "news.example".into(),
            back,
            refused: Some(problem),
            ..StripState::default()
        });
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, 1440.0, 900.0, 0);
        (browser, canvas)
    }

    /// Requirement 7.13: the screen names the host the user asked for and
    /// what failed, says there is no way past, and the strip marks the
    /// channel refused.
    #[test]
    fn a_refused_certificate_shows_why_and_no_way_past() {
        let (_, canvas) = refused(CertificateProblem::Expired, true);
        let texts: Vec<&str> = canvas.texts.iter().map(|(_, text)| text.as_str()).collect();
        let joined = texts.join(" ");
        for expected in [HEADING_TEXT, "news.example", "Certificate refused", "Try again", "Go back", "Close tab"] {
            assert!(joined.contains(expected), "{expected} in {texts:?}");
        }
        assert!(joined.contains("expired") && joined.contains("no way past") && joined.contains("date or time"), "{joined}");
        assert!(!joined.contains("Not secure"), "the refusal, not the plain-connection mark");
        let (_, canvas) = refused(CertificateProblem::WrongHost, false);
        let joined: String = canvas.texts.iter().map(|(_, text)| text.as_str()).collect::<Vec<_>>().join(" ");
        assert!(joined.contains("for another site") && !joined.contains("Go back") && !joined.contains("date or time"), "{joined}");
    }

    /// The buttons are Reload, Back and Close tab: nothing else, and nothing
    /// that opens the channel.
    #[test]
    fn the_screens_buttons_are_reload_back_and_close() {
        let (mut browser, _) = refused(CertificateProblem::UnknownIssuer, true);
        browser.take_outputs();
        for (rect, expected) in
            [(browser.refused.try_again, Output::Reload), (browser.refused.back, Output::Back), (browser.refused.close, Output::CloseTab(0))]
        {
            let point = middle(rect);
            browser.pointer_moved(point.x, point.y, 0);
            browser.pointer_pressed(&mut Recorder::new(), false, 0);
            assert_eq!(browser.take_outputs(), [expected]);
        }
        browser.set_strip(StripState { refused: None, ..browser.strip.clone() });
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        assert!(!browser.certificate_refused() && browser.refused.try_again == Rect::ZERO, "gone once the channel's contents arrive");
    }
}
