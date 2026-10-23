//! Which of the strip's text fields has the keys: the panels in the
//! order their keys are taken, or the address field; and what Copy, Cut
//! and Paste do with it (decision 84).

use alloc::string::String;

use crate::browser::{Browser, Key, Modifiers};
use crate::editor::Editor;

impl Browser {
    /// Closes every panel of the strip, before another opens: one at a time.
    pub(super) fn close_panels(&mut self) {
        if self.star_open() {
            self.star_done();
        }
        if self.settings_open() {
            self.toggle_settings();
        }
        if self.history_open() {
            self.set_history_open(false);
        }
        if self.bookmarks_open() {
            self.toggle_bookmarks();
        }
    }

    /// The text field that has the keys: a panel's, in the order their
    /// keys are taken, or the address field.
    pub(super) fn focused_field(&mut self) -> Option<&mut Editor> {
        if self.star_editing() {
            return self.star_field();
        }
        if self.history_editing() {
            return self.history_field();
        }
        if self.cookies_editing() {
            return self.cookies_field();
        }
        if self.console_searching() {
            return self.console_field();
        }
        if self.bookmarks_editing() {
            return self.bookmarks_field();
        }
        if self.settings_editing() {
            return self.settings_field();
        }
        self.editing.then_some(&mut self.editor)
    }

    /// The selected text of the field with the keys, for Copy (decision
    /// 84); `None` when no field has them or nothing is selected.
    #[must_use]
    pub fn selected_text(&mut self) -> Option<String> {
        let field = self.focused_field()?;
        let (start, end) = field.selection();
        field.text().get(start..end).filter(|text| !text.is_empty()).map(String::from)
    }

    /// Cut: the selected text, as [`Browser::selected_text`], deleted from
    /// its field as Backspace deletes it, so the field's own rules hold.
    pub fn cut(&mut self, now: u64) -> Option<String> {
        let text = self.selected_text()?;
        self.key(Key::Backspace, Modifiers::default(), now);
        Some(text)
    }

    /// Paste: `text` typed into the field with the keys, as typing it
    /// would, so its hidden characters are dropped and its length bounded.
    /// Nothing happens when no field has the keys. Returns whether the
    /// window needs drawing again.
    pub fn paste_text(&mut self, text: &str, now: u64) -> bool {
        if self.focused_field().is_none() {
            return false;
        }
        self.key(Key::Text(text), Modifiers::default(), now)
    }
}
