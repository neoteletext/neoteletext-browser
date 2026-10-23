//! The clipboard and the menus: the fields' cut, copy and paste, the menu
//! bar's items, and the Dock menu's choices.

use dock_menu::{Choice, EditAction, MenuAction, MenuItem, ShortcutKey};
use interface_core::browser::{Key as BrowserKey, Modifiers};
use protocol::ToCore;
use winit::event_loop::ActiveEventLoop;

use super::App;

/// The edit a clipboard key makes with the browser's modifier; Select All
/// is the interface core's own shortcut.
pub(super) fn clipboard_key(character: &str) -> Option<EditAction> {
    match character.to_ascii_lowercase().as_str() {
        "x" => Some(EditAction::Cut),
        "c" => Some(EditAction::Copy),
        "v" => Some(EditAction::Paste),
        _ => None,
    }
}

/// The most of the clipboard any taker uses: a paste to a channel, a
/// PASTE of at most `MAX_PASTE` bytes (the input system's REQ-INT-103),
/// or, built without typing, the longest field, the address field.
#[cfg(feature = "interaction")]
const MAX_CLIPBOARD: usize = neoteletext_input::MAX_PASTE;
#[cfg(not(feature = "interaction"))]
const MAX_CLIPBOARD: usize = interface_core::browser::MAX_ADDRESS;
// Every field takes at most the address field's length, which the
// clipboard's bound must cover.
const _: () = assert!(MAX_CLIPBOARD >= interface_core::browser::MAX_ADDRESS);

/// The clipboard's text as far as any taker uses it, cut on a character
/// boundary as soon as it is read (finding F-54), the rest let go: the
/// clipboard is any program's to fill, and nothing past the bound is
/// kept or passed on.
fn bounded(mut text: String) -> String {
    if text.len() > MAX_CLIPBOARD {
        let end = (0..=MAX_CLIPBOARD).rev().find(|&end| text.is_char_boundary(end)).unwrap_or(0);
        text.truncate(end);
        text.shrink_to_fit();
    }
    text
}

impl App {
    /// The system clipboard, opened when first needed.
    fn clipboard(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        self.clipboard.as_mut()
    }

    /// The clipboard's text; `None` when it holds none.
    pub(super) fn clipboard_text(&mut self) -> Option<String> {
        self.clipboard()?.get_text().ok().map(bounded)
    }

    /// Puts `text` on the clipboard; returns whether it is there.
    fn copy_text(&mut self, text: String) -> bool {
        self.clipboard().is_some_and(|clipboard| clipboard.set_text(text).is_ok())
    }

    /// Cut, Copy, Paste or Select All in the browser's field with the keys
    /// (`menu-bar.md` section 4); a paste while keys go to a channel goes
    /// to the channel (decision 49). Returns whether the window needs
    /// drawing again.
    pub(super) fn edit_field(&mut self, action: EditAction) -> bool {
        let now = self.now();
        match action {
            EditAction::Copy => {
                if let Some(text) = self.view.browser.selected_text() {
                    self.copy_text(text);
                }
                false
            }
            // Removed only once it is on the clipboard.
            EditAction::Cut => {
                let copied = self.view.browser.selected_text().is_some_and(|text| self.copy_text(text));
                copied && self.view.browser.cut(now).is_some()
            }
            EditAction::Paste => {
                #[cfg(feature = "interaction")]
                if self.view.browser.typing_target().is_some() {
                    return self.paste();
                }
                self.clipboard_text().is_some_and(|text| self.view.browser.paste_text(&text, now))
            }
            EditAction::SelectAll => self.view.browser.key(BrowserKey::Text("a"), Modifiers { command: true, ..Modifiers::default() }, now),
        }
    }

    /// An item of the menu bar: the key it names given to the interface
    /// core, so a menu item and its key do the same (requirement 12.5).
    /// Returns whether the window needs drawing again.
    pub(super) fn menu_chosen(&mut self, event_loop: &ActiveEventLoop, item: &MenuItem) -> bool {
        match item.action {
            MenuAction::Browser => {
                let Some(shortcut) = item.shortcut else { return false };
                let key = match shortcut.key {
                    ShortcutKey::Text(text) => BrowserKey::Text(text),
                    ShortcutKey::Left => BrowserKey::Left,
                    ShortcutKey::Right => BrowserKey::Right,
                };
                let modifiers = Modifiers { command: true, shift: shortcut.shift, alt: shortcut.alt, repeat: false };
                let now = self.now();
                self.view.browser.key(key, modifiers, now)
            }
            MenuAction::NewPrivateWindow => {
                self.send(event_loop, &ToCore::NewWindow { private: true });
                false
            }
            MenuAction::TabToNewWindow => {
                if let Some(message) = self.tab_to_new_window() {
                    self.send(event_loop, &message);
                }
                false
            }
            MenuAction::Edit(action) => self.edit_field(action),
            // The system answers its own items; they never come here.
            MenuAction::System(_) => false,
        }
    }

    /// An item of the Dock menu: a tab chosen, or every tab muted or not.
    pub(super) fn dock_chosen(&mut self, event_loop: &ActiveEventLoop, choice: Choice) {
        let message = match choice {
            Choice::Tab(index) => match u8::try_from(index) {
                Ok(index) => ToCore::SelectTab(index),
                Err(_) => return,
            },
            Choice::MuteAll(muted) => ToCore::MuteAll(muted),
        };
        self.send(event_loop, &message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Finding F-54: the clipboard's text is cut to what any taker uses as
    /// soon as it is read, on a character boundary.
    #[test]
    fn the_clipboard_is_cut_as_soon_as_it_is_read() {
        let huge = "é".repeat(MAX_CLIPBOARD);
        let cut = bounded(huge);
        assert!(cut.len() <= MAX_CLIPBOARD && cut.len() > MAX_CLIPBOARD - "é".len(), "{}", cut.len());
        assert!(cut.chars().all(|character| character == 'é'), "cut on a character boundary");
        assert!(cut.capacity() <= MAX_CLIPBOARD, "the rest is let go");
        assert_eq!(bounded("short".into()), "short");
    }
}
