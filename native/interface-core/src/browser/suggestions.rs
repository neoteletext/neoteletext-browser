//! The address field's suggestions (`history-and-bookmarks.md` section
//! 2.10, requirement 20.11): asked for as the user types, shown under the
//! field, the field completed inline unless the last edit deleted.

use alloc::string::String;
use alloc::vec::Vec;

use crate::browser::glyphs::{STAR_RADIUS, star};
use crate::browser::popups::{GAP, ROW};
use crate::browser::{Browser, Output};
use crate::canvas::Canvas;
use crate::geometry::{Point, Rect};
use crate::library::Suggestions;
use crate::text::{HAIRLINE, border, left_centred};
use crate::theme::{ADDRESS, BLACK, DIM, ROW_STATUS, RULE, RULE_SOFT, SELECTED, STAR_ON, WHITE};

/// The address field's suggestions' state in the browser.
#[derive(Debug, Default)]
pub(super) struct SuggestionsUi {
    suggestions: Option<Suggestions>,
    chosen: Option<usize>,
    rows: Vec<(u32, Rect)>,
    suggestions_area: Rect,
    /// The last edit deleted text: no inline completion until more is typed.
    deleted: bool,
}

impl Browser {
    /// The address field's text changed by typing: suggestions are asked
    /// for; `deleted` when the change took text away.
    pub(super) fn edited(&mut self, deleted: bool) {
        self.suggestions.deleted = deleted;
        self.suggestions.chosen = None;
        self.outputs.push(Output::Suggest(String::from(self.editor.text())));
    }

    /// Suggestions for what is typed, as the trusted core sent them: shown
    /// if they answer the text as it is, and the field completed inline
    /// unless the last edit deleted (section 2.10).
    pub fn set_suggestions(&mut self, suggestions: Suggestions) {
        if !self.editing || suggestions.text != self.editor.text() {
            return;
        }
        let typed = suggestions.text.len();
        let (start, end) = self.editor.selection();
        let at_end = start == end && end == typed;
        if !self.suggestions.deleted
            && at_end
            && suggestions.inline.len() > typed
            && suggestions.inline.to_lowercase().starts_with(&suggestions.text.to_lowercase())
        {
            self.editor.set(&suggestions.inline);
            self.editor.place(typed, false);
            self.editor.place(suggestions.inline.len(), true);
        }
        self.suggestions.chosen = None;
        self.suggestions.suggestions = (!suggestions.rows.is_empty()).then_some(suggestions);
    }

    /// Up or Down in the address field: the next suggestion is chosen.
    pub(super) fn choose_suggestion(&mut self, down: bool) -> bool {
        let Some(count) = self.suggestions.suggestions.as_ref().map(|suggestions| suggestions.rows.len()).filter(|count| *count > 0) else { return false };
        self.suggestions.chosen = match (self.suggestions.chosen, down) {
            (None, true) => Some(0),
            (None, false) => Some(count.saturating_sub(1)),
            (Some(at), true) => at.checked_add(1).filter(|&next| next < count),
            (Some(at), false) => at.checked_sub(1),
        };
        true
    }

    /// Enter in the address field: the suggestion chosen, if one is.
    pub(super) fn take_chosen_suggestion(&mut self) -> Option<u32> {
        let at = self.suggestions.chosen.take()?;
        let number = self.suggestions.suggestions.as_ref()?.rows.get(at)?.number;
        Some(number)
    }

    /// The field is no longer edited: its suggestions go.
    pub(super) fn close_suggestions(&mut self) {
        self.suggestions.suggestions = None;
        self.suggestions.chosen = None;
    }

    /// The suggestion at `point`, while they show.
    pub(super) fn suggestion_at(&self, point: Point) -> Option<u32> {
        self.suggestions.suggestions.as_ref()?;
        self.suggestions.rows.iter().find(|(_, rect)| rect.contains(point)).map(|&(number, _)| number)
    }

    /// A press on a suggestion: it opens, as the user's own navigation.
    /// Whether `point` is on the address field's open suggestions.
    pub(super) fn over_suggestions(&self, point: Point) -> bool {
        self.suggestions.suggestions.is_some() && self.suggestions.suggestions_area.contains(point)
    }

    pub(super) fn suggestion_press(&mut self, point: Point, new_tab: bool) -> bool {
        if self.suggestions.suggestions.is_none() || !self.suggestions.suggestions_area.contains(point) {
            return false;
        }
        if let Some(&(number, _)) = self.suggestions.rows.iter().find(|(_, rect)| rect.contains(point)) {
            self.stop_editing();
            self.outputs.push(Output::Suggestion { number, new_tab });
        }
        true
    }

    pub(super) fn draw_suggestions<C: Canvas + ?Sized>(&mut self, canvas: &mut C) {
        self.suggestions.rows.clear();
        self.suggestions.suggestions_area = Rect::ZERO;
        let Some(suggestions) = self.suggestions.suggestions.clone().filter(|_| self.editing) else { return };
        let field = self.regions.field;
        #[allow(clippy::cast_precision_loss, reason = "at most eight rows")]
        let area = Rect::new(field.x0, field.y1 + HAIRLINE, field.x1, field.y1 + HAIRLINE + ROW * suggestions.rows.len() as f64);
        canvas.fill(area, 0.0, BLACK);
        border(canvas, area, HAIRLINE, RULE);
        let mut y = area.y0;
        for (at, row) in suggestions.rows.iter().enumerate() {
            let rect = Rect::new(area.x0, y, area.x1, y + ROW);
            if self.suggestions.chosen == Some(at) {
                canvas.fill(rect, 0.0, SELECTED);
            } else if rect.contains(self.pointer) {
                canvas.fill(rect, 0.0, RULE_SOFT);
            }
            if row.bookmarked {
                star(canvas, Point::new(rect.x0 + GAP + STAR_RADIUS, f64::midpoint(rect.y0, rect.y1)), STAR_ON);
            }
            // The address as the address rules print it, the title lighter
            // beside it, never in its place (requirement 20.13).
            let middle = f64::midpoint(rect.x0, rect.x1);
            canvas.push_clip(Rect::new(rect.x0, y, middle, y + ROW));
            left_centred(canvas, rect, GAP * 4.0, &row.address, ADDRESS, WHITE);
            canvas.pop_clip();
            canvas.push_clip(Rect::new(middle + GAP, y, rect.x1, y + ROW));
            left_centred(canvas, Rect::new(middle + GAP, y, rect.x1, y + ROW), 0.0, &row.title, ROW_STATUS, DIM);
            canvas.pop_clip();
            self.suggestions.rows.push((row.number, rect));
            y += ROW;
        }
        self.suggestions.suggestions_area = area;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{Frame, Key, Modifiers, StripState};
    use crate::library::Suggestion;
    use crate::test_canvas::Recorder;

    fn browser() -> Browser {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
        browser.set_strip(StripState { address: "https://news.example/".into(), back: true, forward: true, ..StripState::default() });
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        browser
    }

    /// Requirement 20.11: typing asks; the suggestions answer; the field is
    /// completed inline, not after a deletion; Down and Enter open one.
    #[test]
    fn typing_asks_for_suggestions_and_completes_inline() {
        let mut browser = browser();
        browser.key(Key::Text("l"), Modifiers { command: true, ..Modifiers::default() }, 0);
        browser.key(Key::Text("ne"), Modifiers::default(), 0);
        assert_eq!(browser.take_outputs(), [Output::TextInput(true), Output::Suggest("ne".into())]);
        let row = Suggestion { number: 9, address: "https://news.example/#302".into(), title: "WEATHER".into(), bookmarked: true };
        browser.set_suggestions(Suggestions { text: "ne".into(), inline: "news.example/".into(), rows: alloc::vec![row.clone()] });
        assert_eq!((browser.editor.text(), browser.editor.selection()), ("news.example/", (2, 13)), "the rest selected");
        browser.key(Key::Backspace, Modifiers::default(), 0);
        browser.take_outputs();
        browser.set_suggestions(Suggestions { text: "ne".into(), inline: "news.example/".into(), rows: alloc::vec![row] });
        assert_eq!(browser.editor.text(), "ne", "no completion after a deletion");
        browser.key(Key::Down, Modifiers::default(), 0);
        browser.key(Key::Enter, Modifiers::default(), 0);
        assert_eq!(browser.take_outputs(), [Output::TextInput(false), Output::Suggestion { number: 9, new_tab: false }]);
    }
}
