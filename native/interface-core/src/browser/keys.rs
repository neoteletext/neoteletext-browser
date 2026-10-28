//! The keys: what the host translates a key into, the modifiers held,
//! and where each key goes, the panels in their order, the address field,
//! links, the page's controls, and the browser's own shortcuts.

use crate::browser::{Browser, Output, WindowAction};

/// A key the browser handles, as the host translates it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key<'a> {
    /// Text typed or composed by an input method.
    Text(&'a str),
    /// Return or Enter.
    Enter,
    /// Escape.
    Escape,
    /// Backspace: deletes before the caret.
    Backspace,
    /// Delete: deletes after the caret.
    Delete,
    /// The left arrow.
    Left,
    /// The right arrow.
    Right,
    /// The up arrow.
    Up,
    /// The down arrow.
    Down,
    /// Home.
    Home,
    /// End.
    End,
    /// Tab: with Shift, backwards.
    Tab,
}

/// The modifier keys. `command` is Command on macOS and Control elsewhere.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools, reason = "independent keys, each held or not, as the platform reports them")]
pub struct Modifiers {
    /// Shift is held.
    pub shift: bool,
    /// Command (macOS) or Control (elsewhere) is held.
    pub command: bool,
    /// Option (macOS) or Alt (elsewhere) is held.
    pub alt: bool,
    /// The key is held down and repeating.
    pub repeat: bool,
}

impl Browser {
    /// Returns whether the window needs drawing again.
    pub fn key(&mut self, key: Key<'_>, modifiers: Modifiers, now: u64) -> bool {
        // In full screen a key the browser keeps shows the strip as it
        // acts, and Escape leaves (decision 85); a channel's keys never
        // come here.
        if self.full_screen() {
            if modifiers.command {
                self.full_screen_key(now);
            } else if key == Key::Escape {
                self.leave_full_screen();
                return true;
            }
        }
        if modifiers.command {
            return self.shortcut(key, modifiers, now);
        }
        // Alt-Home, the Home button's key away from macOS (requirement 19.6).
        if modifiers.alt && key == Key::Home {
            self.outputs.push(Output::Home);
            return true;
        }
        if key == Key::Escape && (self.close_popup() || self.tabs_cancel()) {
            return true;
        }
        if let Some(changed) = self.star_key(key, modifiers.shift) {
            return changed;
        }
        if let Some(changed) = self.history_key(key, modifiers.shift) {
            return changed;
        }
        if let Some(changed) = self.cookies_key(key, modifiers.shift) {
            return changed;
        }
        if let Some(changed) = self.console_key(key, modifiers.shift) {
            return changed;
        }
        if let Some(changed) = self.bookmarks_key(key, modifiers.shift) {
            return changed;
        }
        if let Some(changed) = self.settings_key(key, modifiers.shift) {
            return changed;
        }
        if self.editing {
            return self.edit(key, modifiers.shift);
        }
        #[cfg(feature = "links")]
        if let Some(changed) = self.link_key(key, modifiers, now) {
            return changed;
        }
        if let Some(changed) = self.control_key(key) {
            return changed;
        }
        if let Some(changed) = self.list_key(key, now) {
            return changed;
        }
        match key {
            Key::Escape => return self.wall.unzoom(now),
            Key::Left => self.wall.step(-1, now),
            Key::Right => self.wall.step(1, now),
            Key::Up | Key::Down => self.step_page(key == Key::Down, now),
            Key::Text(text) => {
                for digit in text.chars().filter(char::is_ascii_digit) {
                    self.type_digit(digit);
                }
            }
            _ => return false,
        }
        true
    }

    fn shortcut(&mut self, key: Key<'_>, modifiers: Modifiers, now: u64) -> bool {
        // The browser's Escape: leaves a page whose channel has every
        // other key (INPUT.md section 9.1).
        if key == Key::Escape {
            self.stop_editing();
            return self.wall.unzoom(now);
        }
        // Clear history: Command-Shift-Delete (section 6).
        if matches!(key, Key::Delete | Key::Backspace) && modifiers.shift {
            self.clear_history_choices();
            return true;
        }
        // The tab before or after, shown or not (decision 69): Command-Option
        // and an arrow, Control-Alt elsewhere.
        if modifiers.alt && matches!(key, Key::Left | Key::Right) {
            let count = self.tabs.len();
            if count > 0 {
                let last = count.saturating_sub(1);
                let next =
                    if key == Key::Right { self.active.saturating_add(1).checked_rem(count).unwrap_or(0) } else { self.active.checked_sub(1).unwrap_or(last) };
                self.outputs.push(Output::SelectTab(next));
            }
            return true;
        }
        let Key::Text(text) = key else { return false };
        // Command-1 to 8 choose those tabs, Command-9 the last, as in Chrome.
        if let Some(digit) = text.chars().next().and_then(|first| first.to_digit(10)).filter(|digit| (1..=9).contains(digit) && text.len() == 1) {
            let index = if digit == 9 {
                self.tabs.len().checked_sub(1)
            } else {
                digit.checked_sub(1).and_then(|index| usize::try_from(index).ok()).filter(|index| *index < self.tabs.len())
            };
            if let Some(index) = index {
                self.outputs.push(Output::SelectTab(index));
            }
            return true;
        }
        self.shortcut_letter(text, modifiers, now)
    }

    /// A shortcut of the command key and a letter or sign typed as
    /// `text`: the browser's own, each where the platform's browsers have
    /// it. Returns whether it was one.
    fn shortcut_letter(&mut self, text: &str, modifiers: Modifiers, now: u64) -> bool {
        match text.to_ascii_lowercase().as_str() {
            "l" => self.start_editing(),
            "a" => {
                let Some(field) = self.focused_field() else { return false };
                field.select_all();
            }
            // The settings panel (requirement 19.3), Home (requirement
            // 19.6) and the console (requirement 19.8): Command-comma,
            // Command-Shift-H and Command-Option-I on macOS; Control-comma
            // and Control-Shift-I elsewhere, where Home is Alt-Home.
            "," => self.toggle_settings(),
            "h" if modifiers.shift => self.outputs.push(Output::Home),
            // Reload: Command-R on macOS, Control-R elsewhere.
            // The remote control (decision 68): Command-Shift-R.
            "r" if modifiers.shift => self.toggle_remote(now),
            "r" => self.outputs.push(Output::Reload),
            "i" if modifiers.alt || modifiers.shift => self.toggle_console(),
            // History, bookmarks and private tabs (`history-and-bookmarks.md`
            // section 6): each platform's keys are taken everywhere.
            "y" | "h" => self.toggle_history(),
            "o" if modifiers.shift => self.toggle_bookmarks(),
            "b" if modifiers.alt => self.toggle_bookmarks(),
            "b" if modifiers.shift => self.toggle_bar(),
            // The overview of every page, shown or hidden (decision 109).
            "e" if modifiers.shift => self.toggle_overview_shown(),
            "t" if modifiers.shift => self.outputs.push(Output::History(crate::library::HistoryRequest::Reopen(None))),
            "n" if modifiers.shift => self.outputs.push(Output::NewPrivateTab),
            "t" => self.outputs.push(Output::NewTab),
            "n" => self.outputs.push(Output::NewWindow { private: false }),
            "w" if modifiers.shift => self.outputs.push(Output::Window(WindowAction::Close)),
            "w" => self.outputs.push(Output::CloseTab(self.active)),
            "[" if self.strip.back => self.outputs.push(Output::Back),
            "]" if self.strip.forward => self.outputs.push(Output::Forward),
            "d" => self.outputs.push(Output::Bookmark),
            // The wall of pages and its page size bar, or the page alone
            // (decision 126): Command-G on macOS, Control-Shift-G elsewhere.
            "g" => self.toggle_wall(now),
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::fixture::browser;

    /// Shift-Command-W, Control-Shift-W away from macOS, closes the window
    /// (decision 84); Command-W the tab.
    #[test]
    fn shift_w_closes_the_window() {
        let (mut browser, _) = browser();
        browser.take_outputs();
        browser.key(Key::Text("W"), Modifiers { command: true, shift: true, ..Modifiers::default() }, 0);
        assert_eq!(browser.take_outputs(), [Output::Window(WindowAction::Close)]);
        browser.key(Key::Text("w"), Modifiers { command: true, ..Modifiers::default() }, 0);
        assert_eq!(browser.take_outputs(), [Output::CloseTab(browser.active)]);
    }
}
