//! What the keys do in a text field, each platform as its own text fields
//! do it: every field of the browser takes its keys here, so all of them
//! behave alike. Chrome's address bar and the systems' text fields are the
//! measure, with a word ending at the punctuation of an address.

use crate::browser::{Key, Modifiers};
use crate::editor::Editor;

/// Whose text keys a field follows. Windows and Linux use the same keys
/// and part only over where a word's step to the right lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    /// Option moves by words and Command to the ends; Control has the
    /// system's text keys (Control-A, Control-E, Control-K and their kin).
    Mac,
    /// Control moves by words, and a step right lands on the next word.
    Windows,
    /// Control moves by words, and a step right lands at this word's end.
    Linux,
}

impl Platform {
    /// The platform this is built for.
    pub const CURRENT: Self = if cfg!(target_os = "macos") {
        Self::Mac
    } else if cfg!(windows) {
        Self::Windows
    } else {
        Self::Linux
    };

    /// How far a key goes with `modifiers` held. `command` is Command on
    /// macOS and Control elsewhere ([`Modifiers`]).
    const fn reach(self, modifiers: Modifiers) -> Reach {
        match self {
            Self::Mac if modifiers.command => Reach::Line,
            Self::Mac if modifiers.alt => Reach::Word,
            Self::Windows | Self::Linux if modifiers.command => Reach::Word,
            _ => Reach::Character,
        }
    }
}

/// How far a key moves or deletes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reach {
    Character,
    Word,
    Line,
}

impl Editor {
    /// A key in this field, by the platform's rules.
    pub fn key(&mut self, key: Key<'_>, modifiers: Modifiers) {
        self.key_on(Platform::CURRENT, key, modifiers);
    }

    /// A key in this field by `platform`'s rules. Enter, Escape and Tab are
    /// the field's owner's, and do nothing here.
    pub fn key_on(&mut self, platform: Platform, key: Key<'_>, modifiers: Modifiers) {
        let reach = platform.reach(modifiers);
        let extend = modifiers.shift;
        match key {
            Key::Text(text) if platform == Platform::Mac && modifiers.control => self.control_letter(text, extend),
            Key::Text(text) => self.insert(text),
            Key::Backspace => match reach {
                Reach::Character => self.backspace(),
                Reach::Word => self.delete_to(self.word_before(self.caret)),
                Reach::Line => self.delete_to(0),
            },
            Key::Delete => match reach {
                Reach::Character => self.delete(),
                Reach::Word => self.delete_to(self.word_after(platform, self.caret)),
                Reach::Line => self.delete_to(self.text.len()),
            },
            Key::Left => match reach {
                Reach::Character => self.left(extend),
                Reach::Word => self.move_to(self.word_before(self.start_going_left(extend)), extend),
                Reach::Line => self.home(extend),
            },
            Key::Right => match reach {
                Reach::Character => self.right(extend),
                Reach::Word => self.move_to(self.word_after(platform, self.start_going_right(extend)), extend),
                Reach::Line => self.end(extend),
            },
            Key::Home | Key::Up | Key::PageUp => self.home(extend),
            Key::End | Key::Down | Key::PageDown => self.end(extend),
            Key::Enter | Key::Escape | Key::Tab => {}
        }
    }

    /// macOS's Control keys in any text field, which are the system's and
    /// in no menu (`StandardKeyBinding.dict`). With Shift they extend.
    fn control_letter(&mut self, text: &str, extend: bool) {
        match text.to_ascii_lowercase().as_str() {
            "a" | "p" => self.home(extend),
            "e" | "n" => self.end(extend),
            "b" => self.left(extend),
            "f" => self.right(extend),
            "h" => self.backspace(),
            "d" => self.delete(),
            "k" => self.delete_to(self.text.len()),
            _ => {}
        }
    }

    /// Where a move left starts: a selection being dropped goes from its edge.
    fn start_going_left(&self, extend: bool) -> usize {
        if extend { self.caret } else { self.selection().0 }
    }

    /// Where a move right starts: a selection being dropped goes from its edge.
    fn start_going_right(&self, extend: bool) -> usize {
        if extend { self.caret } else { self.selection().1 }
    }

    fn move_to(&mut self, at: usize, extend: bool) {
        self.place(at, extend);
    }

    /// Deletes the selection, or else from the caret to `at`.
    fn delete_to(&mut self, at: usize) {
        if self.selection().0 == self.selection().1 {
            self.place(at, true);
        }
        self.backspace();
    }

    /// Where the word before `at` starts, past any punctuation between.
    fn word_before(&self, at: usize) -> usize {
        let before = self.text.get(..at).unwrap_or_default();
        let mut start = at;
        let mut in_word = false;
        for (index, character) in before.char_indices().rev() {
            if in_word && !is_word(character) {
                break;
            }
            in_word = in_word || is_word(character);
            start = index;
        }
        start
    }

    /// Where a step right from `at` lands: past the next word on macOS and
    /// Linux, and on the start of the word after on Windows.
    fn word_after(&self, platform: Platform, at: usize) -> usize {
        let after = self.text.get(at..).unwrap_or_default();
        let found = if platform == Platform::Windows {
            let mut left_word = false;
            after.char_indices().find(|&(_, character)| {
                left_word = left_word || !is_word(character);
                left_word && is_word(character)
            })
        } else {
            let mut in_word = false;
            after.char_indices().find(|&(_, character)| {
                let ended = in_word && !is_word(character);
                in_word = in_word || is_word(character);
                ended
            })
        };
        found.map_or(self.text.len(), |(index, _)| at.saturating_add(index))
    }
}

/// A word is a run of letters, digits and underscores; the punctuation of
/// an address (`/`, `.`, `:`) ends one.
pub(super) fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADDRESS: &str = "https://app.neottx.com/home/";
    const COMMAND: Modifiers = Modifiers { shift: false, command: true, alt: false, control: false, repeat: false };
    const OPTION: Modifiers = Modifiers { shift: false, command: false, alt: true, control: false, repeat: false };
    const SHIFT_CONTROL: Modifiers = Modifiers { shift: true, command: false, alt: false, control: true, repeat: false };

    fn shifted(modifiers: Modifiers) -> Modifiers {
        Modifiers { shift: true, ..modifiers }
    }

    /// A field holding the address, the caret at `at` and nothing selected.
    fn field(at: usize) -> Editor {
        let mut editor = Editor::new(ADDRESS.len());
        editor.set(ADDRESS);
        editor.place(at, false);
        editor
    }

    fn selected(editor: &Editor) -> &str {
        let (start, end) = editor.selection();
        editor.text().get(start..end).unwrap_or_default()
    }

    /// Option moves by words on macOS, Control on Windows and Linux, and
    /// with Shift each selects as it goes.
    #[test]
    fn the_word_key_of_each_platform_moves_and_selects_by_words() {
        for (platform, word) in [(Platform::Mac, OPTION), (Platform::Windows, COMMAND), (Platform::Linux, COMMAND)] {
            let mut editor = field(ADDRESS.len());
            editor.key_on(platform, Key::Left, word);
            assert_eq!(editor.caret(), "https://app.neottx.com/".len(), "{platform:?}");
            editor.key_on(platform, Key::Left, shifted(word));
            editor.key_on(platform, Key::Left, shifted(word));
            assert_eq!(selected(&editor), "neottx.com/", "{platform:?}");
        }
    }

    /// A step right past a word: its end on macOS and Linux, and the start
    /// of the next on Windows, as each system's fields do.
    #[test]
    fn a_word_step_right_lands_where_the_platform_lands() {
        let mut mac = field(0);
        mac.key_on(Platform::Mac, Key::Right, OPTION);
        assert_eq!(mac.caret(), "https".len());
        let mut linux = field(0);
        linux.key_on(Platform::Linux, Key::Right, COMMAND);
        assert_eq!(linux.caret(), "https".len());
        let mut windows = field(0);
        windows.key_on(Platform::Windows, Key::Right, COMMAND);
        assert_eq!(windows.caret(), "https://".len());
    }

    /// A word's step stops at the ends of the text.
    #[test]
    fn a_word_step_stops_at_the_ends() {
        for platform in [Platform::Mac, Platform::Windows, Platform::Linux] {
            let word = if platform == Platform::Mac { OPTION } else { COMMAND };
            let mut editor = field(0);
            editor.key_on(platform, Key::Left, word);
            assert_eq!(editor.caret(), 0);
            let mut editor = field(ADDRESS.len());
            editor.key_on(platform, Key::Right, word);
            assert_eq!(editor.caret(), ADDRESS.len());
        }
    }

    /// Without Shift, a word's step leaves a selection from its edge.
    #[test]
    fn a_word_step_drops_a_selection_from_its_edge() {
        let mut editor = field(0);
        editor.select_all();
        editor.key_on(Platform::Mac, Key::Left, OPTION);
        assert_eq!((editor.caret(), selected(&editor)), (0, ""));
    }

    /// Command goes to the ends on macOS, and selects with Shift.
    #[test]
    fn command_and_an_arrow_go_to_the_ends_on_macos() {
        let mut editor = field(5);
        editor.key_on(Platform::Mac, Key::Right, COMMAND);
        assert_eq!(editor.caret(), ADDRESS.len());
        editor.key_on(Platform::Mac, Key::Left, shifted(COMMAND));
        assert_eq!(selected(&editor), ADDRESS);
    }

    /// The deleting keys take a word, or on macOS with Command the line,
    /// and a selection first.
    #[test]
    fn the_deleting_keys_take_what_the_platform_says() {
        for (platform, word) in [(Platform::Mac, OPTION), (Platform::Linux, COMMAND), (Platform::Windows, COMMAND)] {
            let mut editor = field(ADDRESS.len() - 1);
            editor.key_on(platform, Key::Backspace, word);
            assert_eq!(editor.text(), "https://app.neottx.com//", "{platform:?}");
            let mut editor = field(0);
            editor.key_on(platform, Key::Delete, word);
            let left = if platform == Platform::Windows { "app.neottx.com/home/" } else { "://app.neottx.com/home/" };
            assert_eq!(editor.text(), left, "{platform:?}");
            let mut editor = field(0);
            editor.select_word(9);
            editor.key_on(platform, Key::Backspace, word);
            assert_eq!(editor.text(), "https://.neottx.com/home/", "the selection, not a word: {platform:?}");
        }
        let mut editor = field(8);
        editor.key_on(Platform::Mac, Key::Backspace, COMMAND);
        assert_eq!(editor.text(), "app.neottx.com/home/");
    }

    /// macOS's Control keys, which no menu holds.
    #[test]
    fn macos_control_keys_work_in_a_field() {
        let control = Modifiers { control: true, ..Modifiers::default() };
        let mut editor = field(5);
        editor.key_on(Platform::Mac, Key::Text("e"), control);
        assert_eq!(editor.caret(), ADDRESS.len());
        editor.key_on(Platform::Mac, Key::Text("a"), control);
        assert_eq!(editor.caret(), 0);
        editor.key_on(Platform::Mac, Key::Text("f"), control);
        assert_eq!(editor.caret(), 1);
        editor.key_on(Platform::Mac, Key::Text("b"), control);
        assert_eq!(editor.caret(), 0);
        editor.key_on(Platform::Mac, Key::Text("E"), SHIFT_CONTROL);
        assert_eq!(selected(&editor), ADDRESS, "with Shift it selects");
        let mut editor = field("https://".len());
        editor.key_on(Platform::Mac, Key::Text("k"), control);
        assert_eq!(editor.text(), "https://");
        editor.key_on(Platform::Mac, Key::Text("h"), control);
        assert_eq!(editor.text(), "https:/");
        editor.key_on(Platform::Mac, Key::Text("a"), control);
        editor.key_on(Platform::Mac, Key::Text("d"), control);
        assert_eq!(editor.text(), "ttps:/");
    }

    /// Each step works on characters, not bytes.
    #[test]
    fn a_word_is_measured_in_characters() {
        let mut editor = Editor::new(40);
        editor.set("été 東京");
        editor.place(editor.text().len(), false);
        editor.key_on(Platform::Mac, Key::Left, OPTION);
        assert_eq!(editor.text().get(editor.caret()..), Some("東京"));
        editor.key_on(Platform::Mac, Key::Left, OPTION);
        assert_eq!(editor.caret(), 0);
    }
}
