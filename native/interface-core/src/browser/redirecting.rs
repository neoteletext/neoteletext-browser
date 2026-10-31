//! A redirect's notice (REQ-1686): while the strip says a redirect is taking
//! the tab out of its channel, the page area is cleared and says where to,
//! the complete address in lines centred across it and the block centred
//! down it. It asks nothing and has no buttons.

use alloc::vec::Vec;

use super::Browser;
use crate::canvas::{Canvas, Style};
use crate::geometry::Rect;
use crate::text::{centred, wrapped_anywhere};
use crate::theme::{ADDRESS, BLACK, HEADING, WHITE};

/// The notice's first line (REQ-1686 item 2).
const HEADLINE: &str = "Redirecting to:";
/// Between the notice's text and the page area's edges.
const INSET: f64 = 16.0;
/// Between the first line and the address.
const GAP: f64 = 12.0;
/// The address gets smaller by this until it fits (REQ-1686 item 3).
const SIZE_STEP: f32 = 1.0;
/// The size it stops at: a window narrowed past what the smallest window's
/// longest address needs is the only way here, and is clipped.
const SMALLEST_SIZE: f32 = 6.0;

/// The address wrapped to `area`, in the largest style it fits in with the
/// first line above it, with that style's line height.
fn fit<'text, C: Canvas + ?Sized>(canvas: &mut C, address: &'text str, area: Rect) -> (Vec<&'text str>, Style, f64) {
    let room = area.height() - canvas.line_height(HEADING) - GAP;
    let mut style = ADDRESS;
    loop {
        let lines = wrapped_anywhere(canvas, address, area.width(), style);
        let line_height = canvas.line_height(style);
        #[allow(clippy::cast_precision_loss, reason = "a few dozen lines")]
        let height = lines.len() as f64 * line_height;
        if height <= room || style.size <= SMALLEST_SIZE {
            return (lines, style, line_height);
        }
        style.size -= SIZE_STEP;
    }
}

impl Browser {
    /// Clears `area`, the page area, and says where a redirect is taking the
    /// tab. Returns whether it did.
    pub(super) fn draw_redirecting<C: Canvas + ?Sized>(&mut self, canvas: &mut C, area: Rect) -> bool {
        if self.strip.redirecting.is_empty() {
            return false;
        }
        self.redirecting_area = area;
        canvas.fill(area, 0.0, BLACK);
        let inside = Rect::new(area.x0 + INSET, area.y0 + INSET, area.x1 - INSET, area.y1 - INSET);
        let (lines, style, line_height) = fit(canvas, &self.strip.redirecting, inside);
        let headline_height = canvas.line_height(HEADING);
        #[allow(clippy::cast_precision_loss, reason = "a few dozen lines")]
        let height = headline_height + GAP + lines.len() as f64 * line_height;
        let mut y = (f64::midpoint(inside.y0, inside.y1) - height / 2.0).max(inside.y0);
        centred(canvas, Rect::new(inside.x0, y, inside.x1, y + headline_height), HEADLINE, HEADING, WHITE);
        y += headline_height + GAP;
        for line in lines {
            centred(canvas, Rect::new(inside.x0, y, inside.x1, y + line_height), line, style, WHITE);
            y += line_height;
        }
        true
    }
}

#[cfg(test)]
mod tests;
