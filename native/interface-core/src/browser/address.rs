//! The trusted strip's address field: editing it, the keys while it is
//! edited, where a press lands in its text, and how it is drawn, the
//! host in full ink and the rest muted.

use alloc::string::String;

use crate::browser::{Browser, Key, Modifiers, Output};
use crate::canvas::Canvas;
use crate::geometry::Rect;
use crate::theme::{ADDRESS, Colour, DIM, FIELD_INK, FIELD_SELECTION};

pub(super) const FIELD_PADDING: f64 = 12.0;
const CARET_WIDTH: f64 = 2.0;

/// What a file's address begins with, which the field leaves off when it is
/// not being edited.
const FILE_PREFIX: &str = "file://";

/// Whether what was typed names a file, so the interface declares it to the
/// broker (DEC-1551 item 3): the address rules decide what it really is.
#[must_use]
pub fn names_a_file(typed: &str) -> bool {
    let drive = matches!(typed.as_bytes(), [letter, b':', ..] if letter.is_ascii_alphabetic());
    typed.starts_with(FILE_PREFIX) || typed.starts_with(['/', '~', '\\']) || drive
}

impl Browser {
    /// Whether the active tab is private.
    #[must_use]
    pub fn is_private(&self) -> bool {
        self.strip.private
    }

    /// A file dropped on the address field: its address fills the field,
    /// selected, and it goes nowhere until Enter (DEC-1549 item 4).
    pub fn drop_on_address(&mut self, address: &str) {
        self.start_editing();
        self.editor.set(address);
        self.editor.select_all();
    }

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
        self.cancel_field();
        if self.editing {
            self.editing = false;
            self.outputs.push(Output::TextInput(false));
        }
    }

    pub(super) fn edit(&mut self, key: Key<'_>, modifiers: Modifiers) -> bool {
        let before = String::from(self.editor.text());
        match key {
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
            Key::Up | Key::Down if !modifiers.command && self.choose_suggestion(key == Key::Down) => {}
            _ => self.editor.key(key, modifiers),
        }
        // What is typed asks for suggestions (requirement 20.11).
        if self.editing && self.editor.text() != before {
            // Typed over a selection is not a deletion; Backspace and Delete are.
            self.edited(matches!(key, Key::Backspace | Key::Delete));
        }
        true
    }

    /// A press in the address field: the first gives it the focus with
    /// all selected, as browsers do. Where it lands is `field_mouse`'s, as
    /// in every field.
    pub(super) fn focus_address(&mut self) {
        if !self.editing {
            self.start_editing();
            self.field_selected_on_focus();
        }
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
            self.editor.mark_drawn(self.regions.field, x);
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
            // A file shows its path, beside the "File" label; the whole
            // `file:///` address is what is edited (DEC-1549 item 6).
            let address = self.strip.address.strip_prefix(FILE_PREFIX).filter(|_| self.strip.local).unwrap_or(&self.strip.address);
            let identity_at = address.find(self.strip.identity.as_str()).filter(|_| !self.strip.identity.is_empty());
            let mut x = area.x0;
            let parts: [(&str, Colour); 3] = identity_at
                .and_then(|start| {
                    let (before, rest) = address.split_at_checked(start)?;
                    let (identity, after) = rest.split_at_checked(self.strip.identity.len())?;
                    Some([(before, DIM), (identity, FIELD_INK), (after, DIM)])
                })
                .unwrap_or([(address, FIELD_INK), ("", DIM), ("", DIM)]);
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
    use crate::browser::fixture::{HEIGHT, WIDTH, browser, press_at};
    use crate::browser::{Modifiers, StripState};
    use crate::test_canvas::Recorder;

    /// DEC-1551 item 3: what names a file is told apart from a host by its
    /// form alone.
    #[test]
    fn what_names_a_file_is_known_by_its_form() {
        for typed in ["/a/b.t42", "~/b.t42", "file:///a.t42", "C:\\a.tti", "\\\\server\\a"] {
            assert!(names_a_file(typed), "{typed}");
        }
        for typed in ["sport.invalid", "https://a.invalid/", "neoteletext:home"] {
            assert!(!names_a_file(typed), "{typed}");
        }
    }

    /// DEC-1549 items 4 and 6: a file's path shows without its `file://`
    /// until it is edited, whole; a file dropped on the field waits there.
    #[test]
    fn a_file_shows_its_path_and_edits_its_address() {
        let (mut browser, mut canvas) = browser();
        browser.set_strip(StripState { address: "file:///home/a/news.t42".into(), identity: "news.t42".into(), local: true, ..StripState::default() });
        canvas.texts.clear();
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        assert!(canvas.texts.iter().any(|(_, text)| text == "/home/a/"), "the path, without file://: {:?}", canvas.texts);
        assert!(!canvas.texts.iter().any(|(_, text)| text.contains("file://")));
        browser.take_outputs();
        browser.drop_on_address("file:///home/a/other.t42");
        assert_eq!(browser.editor.text(), "file:///home/a/other.t42");
        assert_eq!(browser.take_outputs(), [Output::TextInput(true)], "it goes nowhere until Enter");
    }

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

    /// The address is the active tab's, so a change of tab, by any route,
    /// ends editing what was the other tab's address.
    #[test]
    fn a_change_of_tab_shows_the_new_tabs_address() {
        let (mut browser, mut canvas) = browser();
        browser.set_strip(StripState { address: "https://one.example/".into(), ..browser.strip.clone() });
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let field = browser.regions.field_text;
        press_at(&mut browser, &mut canvas, field);
        browser.pointer_released();
        assert!(browser.editing);
        browser.take_outputs();
        let tabs = browser.tabs.clone();
        browser.set_tabs(tabs, 1);
        browser.set_strip(StripState { address: "https://two.example/".into(), ..browser.strip.clone() });
        assert!(!browser.editing, "the field is not left editing the other tab's address");
        assert_eq!(browser.take_outputs(), [Output::TextInput(false)]);
        let mut drawn = Recorder::new();
        browser.draw(&mut drawn, WIDTH, HEIGHT, 0);
        assert!(drawn.texts.iter().any(|(_, text)| text.contains("two.example")), "the new tab's address shows");
        assert!(!drawn.texts.iter().any(|(_, text)| text.contains("one.example")));
    }

    /// The list of tabs changing without the active tab changing leaves
    /// what is being typed alone: a title arrives at any time.
    #[test]
    fn a_change_to_the_tabs_alone_keeps_editing() {
        let (mut browser, mut canvas) = browser();
        let field = browser.regions.field_text;
        press_at(&mut browser, &mut canvas, field);
        browser.pointer_released();
        let mut tabs = browser.tabs.clone();
        tabs[1].title = "a new title".into();
        browser.set_tabs(tabs, 0);
        assert!(browser.editing);
    }

    /// A press on the tab strip leaves the field, as a press anywhere else
    /// outside it does.
    #[test]
    fn a_press_on_a_tab_ends_editing() {
        let (mut browser, mut canvas) = browser();
        let field = browser.regions.field_text;
        press_at(&mut browser, &mut canvas, field);
        browser.pointer_released();
        assert!(browser.editing);
        let tab = browser.regions.tabs[0].tab;
        press_at(&mut browser, &mut canvas, tab);
        assert!(!browser.editing);
    }

    /// The modifiers reach the field through the browser: with the command
    /// key held these are a text field's keys, not shortcuts, and every
    /// field takes them.
    #[test]
    fn the_moving_keys_reach_a_focused_field_with_the_command_key() {
        let (mut browser, _) = browser();
        browser.set_strip(StripState { address: "https://news.example/".into(), ..browser.strip.clone() });
        let command = Modifiers { command: true, ..Modifiers::default() };
        assert!(!browser.key(Key::Left, command, 0), "no field: no key");
        browser.key(Key::Text("l"), command, 0);
        browser.key(Key::End, Modifiers::default(), 0);
        browser.key(Key::Left, Modifiers { shift: true, ..command }, 0);
        assert_ne!(browser.selected_text(), None, "the command key and an arrow move in the field");
        browser.key(Key::End, Modifiers::default(), 0);
        browser.key(Key::Backspace, command, 0);
        assert_ne!(browser.editor.text(), "https://news.example/", "and delete in it");
        browser.take_outputs();
        browser.key(Key::Left, Modifiers { alt: true, ..command }, 0);
        assert_eq!(browser.take_outputs().len(), 1, "with Option or Alt they stay the tab keys");
    }
}
