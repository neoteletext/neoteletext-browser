//! A tab going to another channel (REQ-7164): while the strip says so, the
//! page area is cleared and says it in its middle, in the browser's own
//! words, which no channel writes or draws over. The trusted core measures
//! the time the words stay; the interface only draws them. They ask nothing
//! and have no buttons.

use super::{Browser, StripState};
use crate::canvas::{Canvas, Style};
use crate::geometry::Rect;
use crate::text::centred;
use crate::theme::{BLACK, HEADING, WHITE};

/// What the page area says (REQ-7164 item 2).
const WORDS: &str = "navigating to new channel";
/// The words' style: a heading's, with their letters as written.
const WORDS_STYLE: Style = Style { caps: false, ..HEADING };

impl Browser {
    /// Whether `strip`, which the trusted core just sent and is not yet
    /// set, clears a page area that still shows a channel: such a strip is
    /// drawn as it arrives, where what else the trusted core changes waits
    /// for the next paced frame. The words of a tab going to another channel
    /// clear it (REQ-7164 item 1: the page area clears at once), and so does
    /// a redirect's notice (REQ-1686 item 5: nothing of the channel being
    /// left stays drawn); a strip that brings either while the other
    /// already shows clears nothing more.
    #[must_use]
    pub fn clears_page_area(&self, strip: &StripState) -> bool {
        let cleared = |strip: &StripState| strip.navigating || !strip.redirecting.is_empty();
        cleared(strip) && !cleared(&self.strip)
    }

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
