//! Small drawing helpers every panel uses.

use crate::canvas::{Canvas, Style};
use crate::geometry::{Point, Rect};
use crate::theme::Colour;

/// Text drawn with no width limit of its own is clipped to this.
const UNBOUNDED: f64 = 10_000.0;
/// One pixel: the width of every rule and outline.
pub const HAIRLINE: f64 = 1.0;

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
