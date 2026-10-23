//! The trusted strip's address field: editing it, the keys while it is
//! edited, where a press lands in its text, and how it is drawn, the
//! host in full ink and the rest muted.

use alloc::string::String;

use crate::browser::{Browser, Key, Output};
use crate::canvas::Canvas;
use crate::geometry::Rect;
use crate::theme::{ADDRESS, Colour, DIM, FIELD_INK, FIELD_SELECTION};

pub(super) const FIELD_PADDING: f64 = 12.0;
const CARET_WIDTH: f64 = 2.0;

impl Browser {
    pub(super) fn start_editing(&mut self) {
        if self.settings_open() {
            self.toggle_settings();
        }
        if !self.editing {
            self.editing = true;
            self.editor.set(&self.strip.address);
            self.field_scroll = 0.0;
            self.outputs.push(Output::TextInput(true));
        }
    }

    pub(super) fn stop_editing(&mut self) {
        self.close_suggestions();
        if self.editing {
            self.editing = false;
            self.outputs.push(Output::TextInput(false));
        }
    }

    pub(super) fn edit(&mut self, key: Key<'_>, shift: bool) -> bool {
        let before = String::from(self.editor.text());
        match key {
            Key::Text(text) => self.editor.insert(text),
            Key::Enter => {
                let chosen = self.take_chosen_suggestion();
                let address = String::from(self.editor.text().trim());
                self.stop_editing();
                if let Some(number) = chosen {
                    self.outputs.push(Output::Suggestion { number, new_tab: false });
                } else if !address.is_empty() {
                    self.outputs.push(Output::Navigate(address));
                }
                return true;
            }
            Key::Escape => self.stop_editing(),
            Key::Tab => return false,
            Key::Backspace => self.editor.backspace(),
            Key::Delete => self.editor.delete(),
            Key::Left => self.editor.left(shift),
            Key::Right => self.editor.right(shift),
            Key::Up | Key::Down if self.choose_suggestion(key == Key::Down) => {}
            Key::Home | Key::Up => self.editor.home(shift),
            Key::End | Key::Down => self.editor.end(shift),
        }
        // What is typed asks for suggestions (requirement 20.11).
        if self.editing && self.editor.text() != before {
            // Typed over a selection is not a deletion; Backspace and Delete are.
            self.edited(matches!(key, Key::Backspace | Key::Delete));
        }
        true
    }

    /// The byte offset in the address nearest to `x`.
    pub(super) fn offset_at<C: Canvas + ?Sized>(&self, canvas: &mut C, x: f64) -> usize {
        let text = self.editor.text();
        let target = x - self.regions.field_text.x0 + self.field_scroll;
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

    pub(super) fn draw_address<C: Canvas + ?Sized>(&mut self, canvas: &mut C, area: Rect) {
        let line = canvas.line_height(ADDRESS);
        let y = (area.y0 + area.y1 - line) / 2.0;
        canvas.push_clip(area);
        if self.editing {
            let text = self.editor.text();
            let caret = canvas.text_width(text.get(..self.editor.caret()).unwrap_or_default(), ADDRESS);
            // Keep the caret in view.
            let room = area.width() - CARET_WIDTH;
            if caret - self.field_scroll > room {
                self.field_scroll = caret - room;
            } else if caret < self.field_scroll {
                self.field_scroll = caret;
            }
            let x = area.x0 - self.field_scroll;
            let (start, end) = self.editor.selection();
            if start != end {
                let from = canvas.text_width(text.get(..start).unwrap_or_default(), ADDRESS);
                let to = canvas.text_width(text.get(..end).unwrap_or_default(), ADDRESS);
                canvas.fill(Rect::new(x + from, y, x + to, y + line), 0.0, FIELD_SELECTION);
            }
            canvas.text(Rect::new(x, y, area.x1 + self.field_scroll, y + line), text, ADDRESS, FIELD_INK);
            canvas.fill(Rect::new(x + caret, y, x + caret + CARET_WIDTH, y + line), 0.0, FIELD_INK);
        } else {
            // The address as the trusted core formatted it, its host in
            // full ink and the rest muted, as browsers show it.
            let address = &self.strip.address;
            let identity_at = address.find(self.strip.identity.as_str()).filter(|_| !self.strip.identity.is_empty());
            let mut x = area.x0;
            let parts: [(&str, Colour); 3] = identity_at
                .and_then(|start| {
                    let (before, rest) = address.split_at_checked(start)?;
                    let (identity, after) = rest.split_at_checked(self.strip.identity.len())?;
                    Some([(before, DIM), (identity, FIELD_INK), (after, DIM)])
                })
                .unwrap_or([(address.as_str(), FIELD_INK), ("", DIM), ("", DIM)]);
            for (part, colour) in parts {
                canvas.text(Rect::new(x, y, area.x1, y + line), part, ADDRESS, colour);
                x += canvas.text_width(part, ADDRESS);
            }
        }
        canvas.pop_clip();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::fixture::{browser, press_at};
    use crate::browser::{Modifiers, StripState};

    #[test]
    fn typing_an_address_navigates() {
        let (mut browser, mut canvas) = browser();
        browser.take_outputs();
        let field = browser.regions.field;
        press_at(&mut browser, &mut canvas, field);
        browser.key(Key::Text("sport.invalid"), Modifiers::default(), 0);
        browser.key(Key::Enter, Modifiers::default(), 0);
        // What is typed asks for suggestions on the way (requirement 20.11).
        assert_eq!(
            browser.take_outputs(),
            [Output::TextInput(true), Output::Suggest("sport.invalid".into()), Output::TextInput(false), Output::Navigate("sport.invalid".into())]
        );
    }

    /// Requirement 12.11 (decision 84): the field with the keys copies, cuts
    /// and takes a paste as typing, hidden characters dropped; with no field
    /// focused, nothing is copied and a paste goes nowhere, not to the page.
    #[test]
    fn the_focused_field_copies_cuts_and_pastes() {
        let (mut browser, _) = browser();
        browser.set_strip(StripState { address: "https://news.example/".into(), ..browser.strip.clone() });
        browser.take_outputs();
        assert_eq!(browser.selected_text(), None);
        assert!(!browser.paste_text("101", 0) && browser.digits.is_empty(), "no field: the page's number keys are not typed");
        let command = Modifiers { command: true, ..Modifiers::default() };
        browser.key(Key::Text("l"), command, 0);
        assert_eq!(browser.selected_text().as_deref(), Some("https://news.example/"), "Open Location selects the address");
        browser.key(Key::End, Modifiers::default(), 0);
        assert_eq!(browser.selected_text(), None, "nothing selected");
        assert!(browser.paste_text("1\u{202E}0\n0", 0));
        assert_eq!(browser.editor.text(), "https://news.example/100", "hidden characters dropped");
        browser.key(Key::Text("a"), command, 0);
        assert_eq!(browser.cut(0).as_deref(), Some("https://news.example/100"));
        assert_eq!(browser.editor.text(), "", "cut removes it");
        browser.key(Key::Escape, Modifiers::default(), 0);
        assert!(!browser.key(Key::Text("a"), command, 0), "select all with no field is no shortcut");
    }
}
