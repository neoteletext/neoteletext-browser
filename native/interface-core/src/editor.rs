//! A single-line text editor for the address field: a caret, a selection,
//! and the edits an address needs. Positions are byte offsets on character
//! boundaries. Hidden characters (controls, direction overrides, zero-width
//! marks) are never inserted, so what the field shows is what it holds.

use alloc::string::String;

/// Characters never typed into the address: controls, and the invisible
/// characters that reorder or hide text. The bidirectional controls are
/// those of Unicode Standard Annex 9, section 2.
#[must_use]
pub fn hidden(character: char) -> bool {
    character.is_control()
        || matches!(character,
            '\u{061C}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}')
}

pub struct Editor {
    text: String,
    caret: usize,
    /// The other end of the selection; equal to the caret when none.
    anchor: usize,
    limit: usize,
}

impl Editor {
    /// An editor holding at most `limit` bytes.
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self { text: String::new(), caret: 0, anchor: 0, limit }
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// The selected byte range, start first.
    #[must_use]
    pub fn selection(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    /// Replaces the text and selects all of it, as a browser does when the
    /// address field gains focus.
    pub fn set(&mut self, text: &str) {
        self.text.clear();
        self.insert(text);
        self.select_all();
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
    }

    fn delete_selection(&mut self) -> bool {
        let (start, end) = self.selection();
        if start == end {
            return false;
        }
        self.text.replace_range(start..end, "");
        self.caret = start;
        self.anchor = start;
        true
    }

    /// Inserts `text` at the caret, replacing any selection, without its
    /// hidden characters and only as far as the limit allows.
    pub fn insert(&mut self, text: &str) {
        self.delete_selection();
        for character in text.chars().filter(|&character| !hidden(character)) {
            if self.text.len() + character.len_utf8() > self.limit {
                break;
            }
            self.text.insert(self.caret, character);
            self.caret += character.len_utf8();
        }
        self.anchor = self.caret;
    }

    fn previous(&self, at: usize) -> usize {
        self.text[..at].char_indices().next_back().map_or(0, |(index, _)| index)
    }

    fn next(&self, at: usize) -> usize {
        self.text[at..].chars().next().map_or(at, |character| at + character.len_utf8())
    }

    pub fn backspace(&mut self) {
        if !self.delete_selection() && self.caret > 0 {
            let start = self.previous(self.caret);
            self.text.replace_range(start..self.caret, "");
            self.caret = start;
            self.anchor = start;
        }
    }

    pub fn delete(&mut self) {
        if !self.delete_selection() && self.caret < self.text.len() {
            let end = self.next(self.caret);
            self.text.replace_range(self.caret..end, "");
        }
    }

    /// Moves the caret one character, extending the selection when
    /// `extend` is set; without it, a selection collapses to its edge.
    pub fn left(&mut self, extend: bool) {
        let (start, end) = self.selection();
        self.caret = if !extend && start != end { start } else { self.previous(self.caret) };
        if !extend {
            self.anchor = self.caret;
        }
    }

    pub fn right(&mut self, extend: bool) {
        let (start, end) = self.selection();
        self.caret = if !extend && start != end { end } else { self.next(self.caret) };
        if !extend {
            self.anchor = self.caret;
        }
    }

    pub fn home(&mut self, extend: bool) {
        self.caret = 0;
        if !extend {
            self.anchor = 0;
        }
    }

    pub fn end(&mut self, extend: bool) {
        self.caret = self.text.len();
        if !extend {
            self.anchor = self.caret;
        }
    }

    /// Puts the caret at `at`, rounded down to a character boundary.
    pub fn place(&mut self, at: usize, extend: bool) {
        let mut at = at.min(self.text.len());
        while !self.text.is_char_boundary(at) {
            at -= 1;
        }
        self.caret = at;
        if !extend {
            self.anchor = at;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_replaces_the_selection_and_skips_hidden_characters() {
        let mut editor = Editor::new(64);
        editor.set("https://old.example/");
        editor.insert("new\u{202E}.exa\u{200B}mple");
        assert_eq!(editor.text(), "new.example");
        assert_eq!(editor.caret(), editor.text().len());
    }

    #[test]
    fn editing_moves_by_whole_characters() {
        let mut editor = Editor::new(64);
        editor.insert("aé🙂");
        editor.backspace();
        assert_eq!(editor.text(), "aé");
        editor.left(false);
        editor.delete();
        assert_eq!(editor.text(), "a");
        editor.home(false);
        editor.right(true);
        assert_eq!(editor.selection(), (0, 1));
        editor.insert("b");
        assert_eq!(editor.text(), "b");
    }

    #[test]
    fn the_limit_is_never_passed() {
        let mut editor = Editor::new(4);
        editor.insert("abcé");
        assert_eq!(editor.text(), "abc", "é would make five bytes");
        editor.place(99, false);
        assert_eq!(editor.caret(), 3);
    }
}
