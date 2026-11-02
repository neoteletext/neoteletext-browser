//! What the window's own parts draw on below the trusted strip (REQ-740):
//! one value for a frame, worked out once from the open page's screen colour
//! while a page shows alone, and otherwise the browser's own. A channel
//! chooses the colour and nothing else: every text and mark on it is the
//! browser's, chosen here to read on it.

use crate::canvas::Canvas;
use crate::geometry::Rect;
use crate::text::border;
use crate::theme::{CELL_AVAILABLE, CELL_EMPTY, Colour, MARK_CONTRAST, MUTED, PANEL, RULE, RULE_SOFT, WHITE, clearest_ink, contrast, mix};

/// How far secondary text goes from the ink towards the surface: as far as
/// keeps [`MARK_CONTRAST`] on all 4,096 colours a Colour Map entry can hold,
/// where the ink alone may have only 4.58 to spend. At 0.25 the worst of
/// them, 0xB0F, falls to 2.97.
const MUTED_SHARE: f64 = 0.2;
/// How far a panel's outline and the rules go from the ink towards the
/// surface, and the lighter lines between rows.
const RULE_SHARE: f64 = 0.6;
const RULE_SOFT_SHARE: f64 = 0.8;
/// How far an overview cell of a page not yet sent goes from the ink towards
/// the surface, and of a page the channel lacks.
const CELL_AVAILABLE_SHARE: f64 = 0.6;
const CELL_EMPTY_SHARE: f64 = 0.85;
/// The edge of a row whose own fill cannot be told from the surface.
const ROW_EDGE: f64 = 2.0;

/// The colours the window's panels draw with this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    /// What everything stands on.
    pub back: Colour,
    /// Text and marks.
    pub ink: Colour,
    /// Secondary text.
    pub muted: Colour,
    /// Panel outlines, headings' rules and the scroll bar.
    pub rule: Colour,
    /// The lighter lines between rows and around unselected magazines.
    pub rule_soft: Colour,
    /// An overview cell for a page the channel lacks.
    pub cell_empty: Colour,
    /// An overview cell for a page the channel has but has not yet sent.
    pub cell_available: Colour,
}

impl Surface {
    /// The browser's own: the panels of the owner's mockups, on the wall of
    /// pages, before a page has arrived, and on a black page.
    pub const BROWSER: Self =
        Self { back: PANEL, ink: WHITE, muted: MUTED, rule: RULE, rule_soft: RULE_SOFT, cell_empty: CELL_EMPTY, cell_available: CELL_AVAILABLE };

    /// The surface on a page's screen colour. Black is Colour Map entry 0,
    /// which a page with no colour of its own has, and its lists read as
    /// the browser's own do (REQ-740 item 7).
    #[must_use]
    pub fn on(screen: Colour) -> Self {
        if screen == Self::BROWSER.back {
            return Self::BROWSER;
        }
        let ink = clearest_ink(screen);
        Self {
            back: screen,
            ink,
            muted: mix(ink, screen, MUTED_SHARE),
            rule: mix(ink, screen, RULE_SHARE),
            rule_soft: mix(ink, screen, RULE_SOFT_SHARE),
            cell_empty: mix(ink, screen, CELL_EMPTY_SHARE),
            cell_available: mix(ink, screen, CELL_AVAILABLE_SHARE),
        }
    }

    /// Whether `colour` can be told from the surface. The browser's own
    /// colours were chosen together, its dark fills to sit quietly on black.
    fn shows(self, colour: Colour) -> bool {
        self == Self::BROWSER || contrast(colour, self.back) >= MARK_CONTRAST
    }

    /// `colour` as a mark on the surface: itself while it shows, and the ink
    /// on a surface it would vanish on, as a magazine's yellow on yellow.
    #[must_use]
    pub fn mark(self, colour: Colour) -> Colour {
        if self.shows(colour) { colour } else { self.ink }
    }

    /// Edges `rect`, already filled with `fill`, in the ink when that fill
    /// cannot be told from the surface, as the chosen row's blue on a blue
    /// screen: the fill stays, for the white text on it.
    pub fn edge<C: Canvas + ?Sized>(self, canvas: &mut C, rect: Rect, fill: Colour) {
        if !self.shows(fill) {
            border(canvas, rect, ROW_EDGE, self.ink);
        }
    }
}

#[cfg(test)]
mod tests;
