//! A tab going to another channel (REQ-7164): while the strip says so, the
//! page area is cleared and says it in its middle, in the browser's own
//! words, which no channel writes or draws over. The trusted core measures
//! the time the words stay; the interface only draws them. They ask nothing
//! and have no buttons.

use super::Browser;
use crate::canvas::{Canvas, Style};
use crate::geometry::Rect;
use crate::text::centred;
use crate::theme::{BLACK, HEADING, WHITE};

/// What the page area says (REQ-7164 item 2).
const WORDS: &str = "navigating to new channel";
/// The words' style: a heading's, with their letters as written.
const WORDS_STYLE: Style = Style { caps: false, ..HEADING };

impl Browser {
    /// Clears `area`, the page area, and says in its middle that the tab is
    /// going to another channel.
    pub(super) fn draw_navigating<C: Canvas + ?Sized>(&mut self, canvas: &mut C, area: Rect) {
        self.navigating_area = area;
        canvas.fill(area, 0.0, BLACK);
        centred(canvas, area, WORDS, WORDS_STYLE, WHITE);
    }
}

#[cfg(test)]
mod tests;
