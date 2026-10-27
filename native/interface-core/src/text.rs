//! Small drawing helpers every panel uses.

use alloc::string::String;
use alloc::vec::Vec;

use crate::canvas::{Canvas, Style};
use crate::geometry::{Point, Rect};
use crate::theme::Colour;

/// Text drawn with no width limit of its own is clipped to this.
const UNBOUNDED: f64 = 10_000.0;
/// One pixel: the width of every rule and outline.
pub const HAIRLINE: f64 = 1.0;
/// What ends text cut to fit its place.
const ELLIPSIS: char = '\u{2026}';

/// As much of `text` as fits in `width`, ending in an ellipsis when cut,
/// so a title narrowed by its column shows whole letters and says it goes
/// on, rather than stopping mid-letter at a clip.
pub fn fitted<C: Canvas + ?Sized>(canvas: &mut C, text: &str, width: f64, style: Style) -> String {
    if canvas.text_width(text, style) <= width {
        return String::from(text);
    }
    let boundaries: Vec<usize> = text.char_indices().map(|(at, _)| at).collect();
    let cut = |kept: usize| {
        let end = boundaries.get(kept).copied().unwrap_or(text.len());
        let mut shown = String::from(text.get(..end).unwrap_or("").trim_end());
        shown.push(ELLIPSIS);
        shown
    };
    // The most characters kept that fit with the ellipsis: widths only grow
    // as characters are added, so a binary search finds it.
    let (mut low, mut high) = (0, boundaries.len().saturating_sub(1));
    while low < high {
        let middle = low.saturating_add(high).div_ceil(2);
        if canvas.text_width(&cut(middle), style) <= width {
            low = middle;
        } else {
            high = middle.saturating_sub(1);
        }
    }
    let shown = cut(low);
    if canvas.text_width(&shown, style) <= width { shown } else { String::new() }
}

/// Text from `rect`'s top-left, fitted to its width with an ellipsis.
pub fn fitted_text<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect, text: &str, style: Style, colour: Colour) {
    let shown = fitted(canvas, text, rect.width(), style);
    canvas.text(rect, &shown, style, colour);
}

/// Text vertically centred in `rect` from its left edge plus `inset`,
/// fitted to the rest of its width with an ellipsis.
pub fn left_fitted<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect, inset: f64, text: &str, style: Style, colour: Colour) {
    let shown = fitted(canvas, text, rect.width() - inset, style);
    left_centred(canvas, rect, inset, &shown, style, colour);
}

/// Text with its top-left corner at `at`.
pub fn label<C: Canvas + ?Sized>(canvas: &mut C, at: Point, text: &str, style: Style, colour: Colour) {
    canvas.text(Rect::new(at.x, at.y, at.x + UNBOUNDED, at.y + UNBOUNDED), text, style, colour);
}

/// Text centred in `rect`, clipped to it.
pub fn centred<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect, text: &str, style: Style, colour: Colour) {
    let width = canvas.text_width(text, style);
    let height = canvas.line_height(style);
    let x = (rect.x0 + rect.x1 - width) / 2.0;
    let y = (rect.y0 + rect.y1 - height) / 2.0;
    canvas.text(Rect::new(x.max(rect.x0), y, rect.x1, rect.y1.max(y + height)), text, style, colour);
}

/// Text vertically centred in `rect`, from its left edge plus `inset`,
/// clipped to the rectangle.
pub fn left_centred<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect, inset: f64, text: &str, style: Style, colour: Colour) {
    let height = canvas.line_height(style);
    let y = (rect.y0 + rect.y1 - height) / 2.0;
    canvas.text(Rect::new(rect.x0 + inset, y, rect.x1, rect.y1.max(y + height)), text, style, colour);
}

/// Text vertically centred in `rect`, ending `inset` before its right edge.
pub fn right_centred<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect, inset: f64, text: &str, style: Style, colour: Colour) {
    let width = canvas.text_width(text, style);
    let height = canvas.line_height(style);
    let y = (rect.y0 + rect.y1 - height) / 2.0;
    let x = rect.x1 - inset - width;
    canvas.text(Rect::new(x, y, rect.x1, rect.y1.max(y + height)), text, style, colour);
}

/// A one-pixel outline just inside `rect`.
pub fn outline<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect, colour: Colour) {
    canvas.fill(Rect::new(rect.x0, rect.y0, rect.x1, rect.y0 + HAIRLINE), 0.0, colour);
    canvas.fill(Rect::new(rect.x0, rect.y1 - HAIRLINE, rect.x1, rect.y1), 0.0, colour);
    canvas.fill(Rect::new(rect.x0, rect.y0, rect.x0 + HAIRLINE, rect.y1), 0.0, colour);
    canvas.fill(Rect::new(rect.x1 - HAIRLINE, rect.y0, rect.x1, rect.y1), 0.0, colour);
}

/// An outline `width` pixels thick, inside `rect`.
pub fn border<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect, width: f64, colour: Colour) {
    canvas.fill(Rect::new(rect.x0, rect.y0, rect.x1, rect.y0 + width), 0.0, colour);
    canvas.fill(Rect::new(rect.x0, rect.y1 - width, rect.x1, rect.y1), 0.0, colour);
    canvas.fill(Rect::new(rect.x0, rect.y0, rect.x0 + width, rect.y1), 0.0, colour);
    canvas.fill(Rect::new(rect.x1 - width, rect.y0, rect.x1, rect.y1), 0.0, colour);
}

/// A cross centred on `centre`: two bars corner to corner, each
/// `thickness` wide and reaching `half` from the centre on each axis, as
/// the close and mute glyphs draw it.
pub fn cross<C: Canvas + ?Sized>(canvas: &mut C, centre: Point, half: f64, thickness: f64, colour: Colour) {
    let normal = thickness / 2.0 / core::f64::consts::SQRT_2;
    for direction in [1.0, -1.0] {
        let along = Point::new(half, half * direction);
        let across = Point::new(-normal * direction, normal);
        // A corner, `along` and `across` taken forwards (1) or back (-1).
        let corner = |forward: f64, out: f64| Point::new(centre.x + forward * along.x + out * across.x, centre.y + forward * along.y + out * across.y);
        canvas.polygon(&[corner(-1.0, 1.0), corner(1.0, 1.0), corner(1.0, -1.0), corner(-1.0, -1.0)], colour);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_canvas::Recorder;
    use crate::theme::ROW_TITLE;

    /// The test canvas measures a character as half its size: `ROW_TITLE`'s
    /// characters are a fixed width each, so the cut is exact.
    #[test]
    fn text_too_wide_is_cut_with_an_ellipsis_to_whole_characters() {
        let mut canvas = Recorder::new();
        let character = canvas.text_width("x", ROW_TITLE);
        assert_eq!(fitted(&mut canvas, "Short", character * 10.0, ROW_TITLE), "Short", "what fits is whole");
        assert_eq!(
            fitted(&mut canvas, "A robot arm from bits", character * 8.0, ROW_TITLE),
            "A robot\u{2026}",
            "seven characters and the ellipsis, the space before it dropped"
        );
        assert_eq!(fitted(&mut canvas, "Wider", character * 0.5, ROW_TITLE), "", "nothing fits");
    }
}
