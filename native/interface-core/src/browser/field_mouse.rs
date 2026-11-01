//! The mouse in a text field, for every field there is. A field is an
//! [`Editor`] that `focused_field` can return and that says where it was
//! drawn: `widgets::field` says so for every panel's field, and the address
//! field for itself. So a press that gave a field the keys, by whatever panel
//! drew it, lands here afterwards, and a new field has all this by being
//! drawn the way fields are drawn.
//!
//! A press places the caret, Shift extends the selection, a double click
//! selects the word and a drag from it goes by words, a triple click selects
//! it all, and a drag selects from the press to the pointer.

use crate::browser::Browser;
use crate::browser::full_screen::{DOUBLE_CLICK_DISTANCE, DOUBLE_CLICK_MILLISECONDS};
use crate::canvas::Canvas;
use crate::editor::Editor;
use crate::geometry::Point;
use crate::theme::ADDRESS;

/// The clicks of a double click, and of a triple click, after which the
/// next click starts again.
const WORD_CLICKS: u8 = 2;
const MOST_CLICKS: u8 = 3;

/// The main button in a field. There is one pointer, so one of these does
/// for every field.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct FieldMouse {
    /// The last press: where, when and how many clicks it made.
    last: Option<(Point, u64, u8)>,
    /// The selection's fixed end, while a press can drag it.
    anchor: Option<usize>,
    /// Where the pointer went since the last draw, which alone can measure text.
    to: Option<f64>,
    /// The word a double click pressed, while its drag goes by words.
    word: Option<(usize, usize)>,
    /// The button is up, and a last move may still wait for the next draw.
    released: bool,
    /// The press focused the address, which then shows all of it selected.
    selected_on_focus: bool,
}

impl FieldMouse {
    /// The clicks this press makes.
    fn count(&mut self, point: Point, now: u64) -> u8 {
        let again = self.last.filter(|&(at, when, _)| {
            now.saturating_sub(when) <= DOUBLE_CLICK_MILLISECONDS && libm::hypot(point.x - at.x, point.y - at.y) <= DOUBLE_CLICK_DISTANCE
        });
        let clicks = again.map_or(0, |(_, _, clicks)| clicks).checked_rem(MOST_CLICKS).unwrap_or(0).saturating_add(1);
        self.last = Some((point, now, clicks));
        clicks
    }

    /// No drag any more, whatever was waiting.
    fn end(&mut self) {
        *self = Self { last: self.last, ..Self::default() };
    }
}

/// The byte offset in `editor` nearest to `x`, by where it was last drawn.
fn offset_at<C: Canvas + ?Sized>(canvas: &mut C, editor: &Editor, x: f64) -> usize {
    let text = editor.text();
    let target = x - editor.drawn().origin;
    let mut best = (0, f64::MAX);
    for (offset, _) in text.char_indices().chain(core::iter::once((text.len(), ' '))) {
        let before = text.get(..offset).unwrap_or_default();
        let distance = (canvas.text_width(before, ADDRESS) - target).abs();
        if distance < best.1 {
            best = (offset, distance);
        }
    }
    best.0
}

/// Selects from `anchor` to `at`: from the word pressed to the word at `at`
/// when the press was a double click.
fn select_to(editor: &mut Editor, anchor: usize, word: Option<(usize, usize)>, at: usize) {
    let Some((word_start, word_end)) = word else {
        editor.place(anchor, false);
        editor.place(at, true);
        return;
    };
    let (start, end) = editor.word_at(at);
    if at < word_start {
        editor.place(word_end, false);
        editor.place(start, true);
    } else {
        editor.place(word_start, false);
        editor.place(end.max(word_end), true);
    }
}

impl Browser {
    /// After a press has gone to whatever it was on: when it landed in the
    /// field that has the keys, the caret goes there. Returns whether it did.
    pub(super) fn press_field<C: Canvas + ?Sized>(&mut self, canvas: &mut C, extend: bool, now: u64) -> bool {
        let point = self.pointer;
        let selected_on_focus = self.field_mouse.selected_on_focus;
        self.field_mouse.end();
        let at = match self.focused_field() {
            Some(editor) if editor.drawn().area.contains(point) => offset_at(canvas, editor, point.x),
            _ => return false,
        };
        let clicks = self.field_mouse.count(point, now);
        let Some(editor) = self.focused_field() else { return false };
        let (anchor, word) = if selected_on_focus {
            (Some(at), None)
        } else if clicks == WORD_CLICKS {
            let word = editor.word_at(at);
            editor.select_word(at);
            (Some(word.0), Some(word))
        } else if clicks == MOST_CLICKS {
            editor.select_all();
            (None, None)
        } else {
            editor.place(at, extend);
            (Some(editor.anchor()), None)
        };
        self.field_mouse.anchor = anchor;
        self.field_mouse.word = word;
        true
    }

    /// The press focused the address: all of it shows selected until a drag.
    pub(super) fn field_selected_on_focus(&mut self) {
        self.field_mouse.selected_on_focus = true;
    }

    /// The pointer moved with the button held after a press in a field: the
    /// selection follows it at the next draw. Returns whether it did.
    pub(super) fn drag_field(&mut self, x: f64) -> bool {
        if self.field_mouse.anchor.is_none() || self.field_mouse.released {
            return false;
        }
        self.field_mouse.to = Some(x);
        true
    }

    /// The main button was released. A move not drawn yet still counts: the
    /// drag ends after the next draw has taken it.
    pub(super) fn release_field(&mut self) {
        self.field_mouse.released = true;
    }

    /// The field lost the keys: nothing of a drag is left.
    pub(super) fn cancel_field(&mut self) {
        self.field_mouse.end();
    }

    /// Before a draw: the selection takes the pointer's last place.
    pub(super) fn follow_field_drag<C: Canvas + ?Sized>(&mut self, canvas: &mut C) {
        let FieldMouse { anchor, to, word, released, .. } = self.field_mouse;
        if let (Some(anchor), Some(x)) = (anchor, to)
            && let Some(editor) = self.focused_field()
        {
            let at = offset_at(canvas, editor, x);
            select_to(editor, anchor, word, at);
        }
        self.field_mouse.to = None;
        if released {
            self.field_mouse.end();
        }
    }
}

#[cfg(test)]
pub(in crate::browser) mod tests {
    #![allow(clippy::arithmetic_side_effects, reason = "a test")]
    use super::*;
    use crate::browser::Modifiers;
    use crate::browser::fixture::{HEIGHT, WIDTH, browser};
    use crate::browser::{Key, StripState};
    use crate::geometry::Rect;
    use crate::test_canvas::Recorder;

    const TEXT: &str = "https://news.example/sport/";

    /// `clicks` presses at `letters` characters into the field's text, from
    /// `now`, the last still held.
    fn click(browser: &mut Browser, canvas: &mut Recorder, area: Rect, letters: f64, clicks: u64, now: u64) {
        // A field with no keys yet is pressed well inside it.
        let focused = browser.focused_field().is_some_and(|editor| editor.drawn().area == area);
        let x = if focused { at(browser, canvas, letters) } else { f64::midpoint(area.x0, area.x1) };
        let y = f64::midpoint(area.y0, area.y1);
        for click in 0..clicks {
            browser.pointer_released();
            browser.draw(canvas, WIDTH, HEIGHT, now + click);
            browser.pointer_moved(x, y, now + click);
            browser.pointer_pressed(canvas, false, now + click);
            browser.draw(canvas, WIDTH, HEIGHT, now + click);
        }
    }

    /// Where `letters` characters into the focused field's text lies.
    fn at(browser: &mut Browser, canvas: &mut Recorder, letters: f64) -> f64 {
        let cell = canvas.text_width("a", ADDRESS);
        browser.focused_field().map_or(0.0, |editor| editor.drawn().origin) + cell * letters
    }

    fn drag(browser: &mut Browser, canvas: &mut Recorder, letters: f64, now: u64) {
        let x = at(browser, canvas, letters);
        browser.pointer_moved(x, 0.0, now);
        browser.draw(canvas, WIDTH, HEIGHT, now);
    }

    /// What the mouse does in any field, however narrow: every field's own
    /// tests go through this, with the place it was drawn.
    pub(in crate::browser) fn a_drag_selects(browser: &mut Browser, canvas: &mut Recorder, area: Rect) {
        click(browser, canvas, area, 9.5, 1, 1_000);
        browser.pointer_released();
        browser.draw(canvas, WIDTH, HEIGHT, 1_000);
        browser.focused_field().expect("a field has the keys").place(0, false);
        assert_eq!(browser.selected_text(), None);
        browser.focused_field().expect("a field has the keys").set(TEXT);
        browser.focused_field().expect("a field has the keys").place(0, false);
        browser.draw(canvas, WIDTH, HEIGHT, 2_000);
        click(browser, canvas, area, 0.1, 1, 3_000);
        drag(browser, canvas, 5.0, 3_016);
        assert_eq!(browser.selected_text().as_deref(), Some("https"), "a drag selects from the press, the first letter too");
        browser.pointer_released();
        browser.draw(canvas, WIDTH, HEIGHT, 3_032);
    }

    /// The rest of what the mouse does, in a field wide enough for the text.
    pub(in crate::browser) fn the_mouse_selects(browser: &mut Browser, canvas: &mut Recorder, area: Rect) {
        a_drag_selects(browser, canvas, area);
        click(browser, canvas, area, 9.5, 2, 5_000);
        assert_eq!(browser.selected_text().as_deref(), Some("news"), "a double click selects the word");
        drag(browser, canvas, 15.5, 5_016);
        assert_eq!(browser.selected_text().as_deref(), Some("news.example"), "a drag from it goes by words");
        drag(browser, canvas, 1.5, 5_032);
        assert_eq!(browser.selected_text().as_deref(), Some("https://news"), "and back, keeping the word pressed");
        browser.pointer_released();
        browser.draw(canvas, WIDTH, HEIGHT, 5_048);
        click(browser, canvas, area, 9.5, 3, 7_000);
        assert_eq!(browser.selected_text().as_deref(), Some(TEXT), "a triple click selects it all");
        click(browser, canvas, area, 9.5, 1, 7_003);
        assert_eq!(browser.selected_text(), None, "a fourth press starts again");
        browser.pointer_released();
    }

    #[test]
    fn the_mouse_selects_in_the_address() {
        let (mut browser, mut canvas) = browser();
        browser.set_strip(StripState { address: TEXT.into(), ..browser.strip.clone() });
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let field = browser.regions.field;
        click(&mut browser, &mut canvas, field, 1.0, 1, 0);
        browser.pointer_released();
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        assert_eq!(browser.selected_text().as_deref(), Some(TEXT), "the first press selects it all, as browsers do");
        the_mouse_selects(&mut browser, &mut canvas, field);
    }

    /// A move the last draw has not seen still counts when the button comes up.
    #[test]
    fn a_drag_ended_before_a_draw_keeps_its_last_move() {
        let (mut browser, mut canvas) = browser();
        browser.set_strip(StripState { address: TEXT.into(), ..browser.strip.clone() });
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let field = browser.regions.field;
        click(&mut browser, &mut canvas, field, 1.0, 1, 0);
        click(&mut browser, &mut canvas, field, 5.0, 1, 5_000);
        let x = at(&mut browser, &mut canvas, 0.0);
        browser.pointer_moved(x, 0.0, 5_001);
        browser.pointer_released();
        browser.draw(&mut canvas, WIDTH, HEIGHT, 5_002);
        assert_eq!(browser.selected_text().as_deref(), Some("https"));
    }

    /// A press in a field that has not the keys, or outside the one that
    /// has, moves no caret.
    #[test]
    fn a_press_elsewhere_moves_no_caret() {
        let (mut browser, mut canvas) = browser();
        browser.key(Key::Text("l"), Modifiers { command: true, ..Modifiers::default() }, 0);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let selected = browser.editor.selection();
        browser.pointer_moved(5.0, HEIGHT - 5.0, 0);
        assert!(!browser.press_field(&mut canvas, false, 0));
        assert_eq!(browser.editor.selection(), selected);
    }
}
