//! A single-line text editor for the address field: a caret, a selection,
//! and the edits an address needs. Positions are byte offsets on character
//! boundaries. Hidden characters (controls, direction overrides, zero-width
//! and other invisible characters) are never inserted, so what the field shows is what it holds.

use alloc::string::String;

/// Characters never typed into the address or shown from untrusted text:
/// controls, and the invisible characters that reorder or hide text
/// (finding F-36). These are, by the Unicode Character Database 16.0:
/// every format character (general category Cf), including the
/// bidirectional controls of Unicode Standard Annex 9 section 2 and the
/// tag characters; the line and paragraph separators (Zl, Zp); and the
/// other default-ignorable characters of `DerivedCoreProperties.txt`, the
/// combining grapheme joiner, the Hangul fillers, the Khmer inherent
/// vowels, the Mongolian and other variation selectors, and the reserved
/// default-ignorable ranges. The crate has no Unicode tables (no standard
/// library), so the ranges are listed here.
#[must_use]
pub fn hidden(character: char) -> bool {
    character.is_control()
        || matches!(character,
            // Cf, by the database's own list.
            '\u{00AD}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061C}'
            | '\u{06DD}'
            | '\u{070F}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08E2}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{110BD}'
            | '\u{110CD}'
            | '\u{13430}'..='\u{1343F}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0001}'
            | '\u{E0020}'..='\u{E007F}'
            // Zl and Zp.
            | '\u{2028}'..='\u{2029}'
            // Default-ignorable, not Cf.
            | '\u{034F}'
            | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180D}'
            | '\u{180F}'
            | '\u{2065}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFF8}'
            | '\u{E0000}'
            | '\u{E0002}'..='\u{E001F}'
            | '\u{E0080}'..='\u{E0FFF}')
}

/// A single line of editable text with a caret and a selection, as the
/// address field uses it. Positions are byte offsets on character
/// boundaries.
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

    /// The text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The caret's byte offset.
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
        // The caret and selection belong to the old text: left as they were,
        // inserting would delete the old selection from the empty text.
        self.text.clear();
        self.caret = 0;
        self.anchor = 0;
        self.insert(text);
        self.select_all();
    }

    /// Selects all the text, with the caret at its end.
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
            if self.text.len().saturating_add(character.len_utf8()) > self.limit {
                break;
            }
            self.text.insert(self.caret, character);
            self.caret = self.caret.saturating_add(character.len_utf8());
        }
        self.anchor = self.caret;
    }

    fn previous(&self, at: usize) -> usize {
        self.text.get(..at).and_then(|before| before.char_indices().next_back()).map_or(0, |(index, _)| index)
    }

    fn next(&self, at: usize) -> usize {
        self.text.get(at..).and_then(|after| after.chars().next()).map_or(at, |character| at.saturating_add(character.len_utf8()))
    }

    /// Deletes the selection, or else the character before the caret.
    pub fn backspace(&mut self) {
        if !self.delete_selection() && self.caret > 0 {
            let start = self.previous(self.caret);
            self.text.replace_range(start..self.caret, "");
            self.caret = start;
            self.anchor = start;
        }
    }

    /// Deletes the selection, or else the character after the caret.
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

    /// The same as [`Editor::left`], to the right.
    pub fn right(&mut self, extend: bool) {
        let (start, end) = self.selection();
        self.caret = if !extend && start != end { end } else { self.next(self.caret) };
        if !extend {
            self.anchor = self.caret;
        }
    }

    /// Moves the caret to the start, extending the selection when `extend` is set.
    pub fn home(&mut self, extend: bool) {
        self.caret = 0;
        if !extend {
            self.anchor = 0;
        }
    }

    /// Moves the caret to the end, extending the selection when `extend` is set.
    pub fn end(&mut self, extend: bool) {
        self.caret = self.text.len();
        if !extend {
            self.anchor = self.caret;
        }
    }

    /// The end of the selection the caret is not at.
    #[must_use]
    pub fn anchor(&self) -> usize {
        self.anchor
    }

    /// Selects the word around `at`: the run of letters, digits and
    /// underscores it is in or next to, as a double click selects it; the
    /// punctuation of an address (`/`, `.`, `:`) ends a word.
    pub fn select_word(&mut self, at: usize) {
        let at = at.min(self.text.len());
        let is_word = |character: char| character.is_alphanumeric() || character == '_';
        let before = self.text.get(..at).unwrap_or_default();
        let after = self.text.get(at..).unwrap_or_default();
        let start = before.char_indices().rev().take_while(|&(_, character)| is_word(character)).last().map_or(at, |(offset, _)| offset);
        let end = at.saturating_add(after.char_indices().find(|&(_, character)| !is_word(character)).map_or(after.len(), |(offset, _)| offset));
        self.anchor = start;
        self.caret = end;
    }

    /// Puts the caret at `at`, rounded down to a character boundary.
    pub fn place(&mut self, at: usize, extend: bool) {
        let mut at = at.min(self.text.len());
        while !self.text.is_char_boundary(at) {
            at = at.saturating_sub(1);
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
    fn the_field_can_be_set_again_after_its_text_was_selected() {
        let mut editor = Editor::new(64);
        editor.set("http://127.0.0.1:8071/terminal/");
        editor.set("http://127.0.0.1:8071/terminal/");
        assert_eq!((editor.text(), editor.selection()), ("http://127.0.0.1:8071/terminal/", (0, 31)));
        editor.set("a.example");
        assert_eq!((editor.text(), editor.selection()), ("a.example", (0, 9)));
    }

    #[test]
    fn typing_replaces_the_selection_and_skips_hidden_characters() {
        let mut editor = Editor::new(64);
        editor.set("https://old.example/");
        editor.insert("new\u{202E}.exa\u{200B}mple");
        assert_eq!(editor.text(), "new.example");
        assert_eq!(editor.caret(), editor.text().len());
    }

    /// Finding F-36: every format character (Unicode general category
    /// Cf), the line and paragraph separators (Zl, Zp), the variation
    /// selectors, the Hangul fillers and the combining grapheme joiner are
    /// hidden; letters, marks and spaces that show are not.
    #[test]
    fn every_invisible_character_is_hidden() {
        for character in [
            '\u{00AD}',
            '\u{034F}',
            '\u{0600}',
            '\u{0605}',
            '\u{061C}',
            '\u{06DD}',
            '\u{070F}',
            '\u{0890}',
            '\u{08E2}',
            '\u{115F}',
            '\u{1160}',
            '\u{17B4}',
            '\u{180B}',
            '\u{180E}',
            '\u{180F}',
            '\u{200B}',
            '\u{2028}',
            '\u{2029}',
            '\u{202E}',
            '\u{2060}',
            '\u{2066}',
            '\u{206A}',
            '\u{206F}',
            '\u{3164}',
            '\u{FE00}',
            '\u{FE0F}',
            '\u{FEFF}',
            '\u{FFA0}',
            '\u{FFF9}',
            '\u{FFFB}',
            '\u{110BD}',
            '\u{110CD}',
            '\u{13430}',
            '\u{1343F}',
            '\u{1BCA0}',
            '\u{1BCA3}',
            '\u{1D173}',
            '\u{1D17A}',
            '\u{E0001}',
            '\u{E0020}',
            '\u{E007F}',
            '\u{E0100}',
            '\u{E01EF}',
        ] {
            assert!(hidden(character), "U+{:04X}", u32::from(character));
        }
        for character in ['a', 'é', '\u{0301}', ' ', '\u{00A0}', '\u{3000}', '\u{1100}', '\u{AC00}', '\u{FFFD}', '🙂'] {
            assert!(!hidden(character), "U+{:04X}", u32::from(character));
        }
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
