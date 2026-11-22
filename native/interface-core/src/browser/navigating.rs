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
    /// set, is drawn as it arrives, where what else the trusted core changes
    /// waits for the next paced frame. Two kinds of strip are:
    /// - one that clears a page area still showing a channel. The words of
    ///   a tab going to another channel clear it (REQ-7164 item 1: the page
    ///   area clears at once), and so does a redirect's notice (REQ-1686
    ///   item 5: nothing of the channel being left stays drawn); a strip
    ///   that brings either while the other already shows clears nothing
    ///   more;
    /// - one that ends a redirect's notice, whatever it shows next. The
    ///   notice lasts exactly one second (REQ-1686), which the trusted core
    ///   measures: a paced frame would add to that second up to the pace of
    ///   a page that animates (TODO-7232).
    #[must_use]
    pub fn drawn_as_it_arrives(&self, strip: &StripState) -> bool {
        let redirecting = |strip: &StripState| !strip.redirecting.is_empty();
        let cleared = |strip: &StripState| strip.navigating || redirecting(strip);
        let clears = cleared(strip) && !cleared(&self.strip);
        let ends_notice = redirecting(&self.strip) && !redirecting(strip);
        clears || ends_notice
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
