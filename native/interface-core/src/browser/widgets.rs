//! The controls every panel of the trusted strip draws the same way:
//! buttons, text fields, and the panel itself, centred in the window below
//! the strip.

use crate::canvas::{Canvas, Style};
use crate::editor::{Editor, Shown};
use crate::geometry::{Point, Rect};
use crate::text::{border, centred, outline};
use crate::theme::{ADDRESS, BUTTON, Colour, FIELD, FIELD_INK, FIELD_SELECTION, PANEL, ROW_TITLE, RULE, SELECTED, WHITE};

/// A control's corners.
pub(super) const RADIUS: f64 = 4.0;
/// A control's outline.
pub(super) const OUTLINE: f64 = 2.0;
/// A panel's corners.
const PANEL_RADIUS: f64 = 8.0;
/// Text's distance from a field's edges.
const FIELD_PADDING: f64 = 8.0;
const CARET_WIDTH: f64 = 2.0;
/// The thickness of the line under composed text: one logical pixel, the
/// thinnest line every scale draws.
const COMPOSING_UNDERLINE: f64 = 1.0;
/// The text a field shows.
pub(super) const FIELD_TEXT: Style = ADDRESS;

/// A button, filled when `chosen`.
pub(super) fn button<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect, word: &str, chosen: bool) {
    canvas.fill(rect, RADIUS, if chosen { SELECTED } else { BUTTON });
    border(canvas, rect, OUTLINE, if chosen { SELECTED } else { RULE });
    centred(canvas, rect, word, ROW_TITLE, WHITE);
}

/// A text field showing `editor`, with its caret and selection when focused.
/// A field drawn here has the mouse (`field_mouse`) with nothing more to do.
pub(super) fn field<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect, editor: &Editor, focused: bool) {
    canvas.fill(rect, RADIUS, FIELD);
    if focused {
        border(canvas, rect, OUTLINE, SELECTED);
    }
    let shown = editor.shown();
    let text = shown.text.as_str();
    let line = canvas.line_height(FIELD_TEXT);
    let top = (rect.y0 + rect.y1 - line) / 2.0;
    let inner = Rect::new(rect.x0 + FIELD_PADDING, rect.y0, rect.x1 - FIELD_PADDING, rect.y1);
    canvas.push_clip(inner);
    let caret = canvas.text_width(text.get(..shown.caret).unwrap_or_default(), FIELD_TEXT);
    // Keep the caret in view.
    let shift = (caret - inner.width() + CARET_WIDTH).max(0.0);
    let x = inner.x0 - shift;
    editor.mark_drawn(rect, x);
    // While something is composed, the selection is what it will replace,
    // and the composition's mark shows instead.
    if focused && editor.composing().is_empty() {
        let (start, end) = editor.selection();
        if start != end {
            let from = canvas.text_width(text.get(..start).unwrap_or_default(), FIELD_TEXT);
            let to = canvas.text_width(text.get(..end).unwrap_or_default(), FIELD_TEXT);
            canvas.fill(Rect::new(x + from, top, x + to, top + line), 0.0, FIELD_SELECTION);
        }
    }
    canvas.text(Rect::new(x, top, inner.x1 + shift, top + line), text, FIELD_TEXT, FIELD_INK);
    underline_composing(canvas, &shown, x, top + line, FIELD_TEXT);
    if focused {
        let drawn = Rect::new(x + caret, top, x + caret + CARET_WIDTH, top + line);
        canvas.fill(drawn, 0.0, FIELD_INK);
        editor.mark_caret(drawn);
    }
    canvas.pop_clip();
}

/// The line under what an input method is composing (REQ-763), as the
/// systems' own fields mark it, ending at `bottom`.
pub(super) fn underline_composing<C: Canvas + ?Sized>(canvas: &mut C, shown: &Shown, x: f64, bottom: f64, style: Style) {
    let (start, end) = shown.composing;
    if start == end {
        return;
    }
    let from = canvas.text_width(shown.text.get(..start).unwrap_or_default(), style);
    let to = canvas.text_width(shown.text.get(..end).unwrap_or_default(), style);
    canvas.fill(Rect::new(x + from, bottom - COMPOSING_UNDERLINE, x + to, bottom), 0.0, FIELD_INK);
}

/// A panel `width` by `height`, centred in `area` and no larger than it.
pub(super) fn panel_rect(area: Rect, width: f64, height: f64) -> Rect {
    let width = width.min(area.width());
    let height = height.min(area.height());
    let centre = Point::new(f64::midpoint(area.x0, area.x1), f64::midpoint(area.y0, area.y1));
    Rect::new(centre.x - width / 2.0, centre.y - height / 2.0, centre.x + width / 2.0, centre.y + height / 2.0)
}

/// A panel's ground and outline.
pub(super) fn panel<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect) {
    canvas.fill(rect, PANEL_RADIUS, PANEL);
    outline(canvas, rect, RULE);
}

/// A folder's mark and a list's: drawn, since the browser's fonts have no
/// triangles. Half as tall as it is wide when pointing down.
pub(super) const TRIANGLE: f64 = 8.0;

/// A small triangle centred on `centre`, pointing right or down.
pub(super) fn triangle<C: Canvas + ?Sized>(canvas: &mut C, centre: Point, down: bool, colour: Colour) {
    let half = TRIANGLE / 2.0;
    let points = if down {
        [Point::new(centre.x - half, centre.y - half / 2.0), Point::new(centre.x + half, centre.y - half / 2.0), Point::new(centre.x, centre.y + half / 2.0)]
    } else {
        [Point::new(centre.x - half / 2.0, centre.y - half), Point::new(centre.x + half / 2.0, centre.y), Point::new(centre.x - half / 2.0, centre.y + half)]
    };
    canvas.polygon(&points, colour);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_canvas::Recorder;

    /// REQ-763 (REQ-2019: every panel's field is drawn here): a panel's
    /// field shows what is composed at its caret, underlined, and says where
    /// its caret is; the text it holds is unchanged.
    #[test]
    fn a_panels_field_shows_what_is_composed() {
        let mut editor = Editor::new(64);
        editor.insert("ab");
        editor.compose("中文", None);
        let mut canvas = Recorder::new();
        let rect = Rect::new(10.0, 10.0, 300.0, 40.0);
        field(&mut canvas, rect, &editor, true);
        assert!(canvas.texts.iter().any(|(_, text)| text == "ab中文"), "{:?}", canvas.texts);
        let width = canvas.text_width("中文", FIELD_TEXT);
        assert!(
            canvas.fills.iter().any(|(fill, _)| (fill.width() - width).abs() < 0.01 && (fill.height() - COMPOSING_UNDERLINE).abs() < 0.01),
            "{:?}",
            canvas.fills
        );
        let caret = editor.drawn().caret;
        assert!((caret.x0 - (editor.drawn().origin + canvas.text_width("ab中文", FIELD_TEXT))).abs() < 0.01, "{caret:?}");
        assert_eq!(editor.text(), "ab");
    }
}
