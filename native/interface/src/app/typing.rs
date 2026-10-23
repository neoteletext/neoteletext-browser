//! Typing into a page (`interaction.md`): keys, pastes and the browser's
//! own modifier while keys go to a channel, and the typing state the
//! trusted core sends.

use winit::event::ElementState;
use winit::keyboard::Key;

use super::App;

impl App {
    /// Whether `event` is the browser's paste shortcut, the browser's
    /// modifier and V, while keys go to a channel.
    pub(super) fn is_paste(&self, event: &winit::event::KeyEvent) -> bool {
        use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
        self.view.browser.typing_target().is_some()
            && self.browser_modifier()
            && matches!(event.key_without_modifiers(), Key::Character(ref key) if key.eq_ignore_ascii_case("v"))
    }

    /// The clipboard's text to the channel, as one PASTE; a clipboard that
    /// holds no text pastes nothing.
    pub(super) fn paste(&mut self) -> bool {
        self.clipboard_text().is_some_and(|text| self.view.browser.terminal_paste(&text))
    }

    /// Whether the browser's own modifier is held: Command on macOS,
    /// Control and Shift elsewhere, the keys a terminal emulator keeps.
    pub(super) fn browser_modifier(&self) -> bool {
        if cfg!(target_os = "macos") { self.modifiers.super_key() } else { self.modifiers.control_key() && self.modifiers.shift_key() }
    }

    /// A key while keys go to a channel: every key but the browser's own
    /// goes, pressed, repeating and released, as the trusted core's
    /// session asks for them (INPUT.md section 9.1). Returns whether the
    /// key was the channel's.
    pub(super) fn terminal_key(&mut self, event: &winit::event::KeyEvent) -> Option<bool> {
        use neoteletext_input::event::Action;
        use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
        if self.view.browser.typing_target().is_none() || self.browser_modifier() {
            return None;
        }
        let now = self.now();
        let action = match (event.state, event.repeat) {
            (ElementState::Pressed, false) => Action::Press,
            (ElementState::Pressed, true) => Action::Repeat,
            (ElementState::Released, _) => Action::Release,
        };
        let unmodified = event.key_without_modifiers();
        let parts = crate::keys::parts(action, &event.logical_key, &unmodified, event.physical_key, event.text.as_deref(), self.modifiers);
        let changed = match parts {
            Some(parts) => self.view.browser.terminal_key(parts.action, parts.key, parts.mods, parts.text.as_deref(), parts.base, now),
            // A key that types several characters: they go as text.
            None => match (&event.text, action) {
                (Some(text), Action::Press) => self.view.browser.terminal_text(text, now),
                _ => false,
            },
        };
        Some(changed)
    }

    /// Whether keys go to the open page's channel, as the trusted core says.
    pub(super) fn show_typing(&mut self, typing: &protocol::interaction::Typing, now: u64) {
        let cursor = typing.cursor.map(|cursor| interface_core::browser::CursorView {
            row: cursor.row,
            column: cursor.column,
            width: cursor.width,
            flags: cursor.flags,
        });
        let view = interface_core::browser::TypingView {
            channel: typing.channel,
            page: typing.page,
            on: typing.on,
            secure: typing.secure,
            refused: typing.refused,
            modes: typing.modes,
            cursor,
            acked: typing.acked,
            round_trip_ms: typing.round_trip_ms,
            show: typing.show,
        };
        self.view.browser.set_typing(view, now);
        self.sync_ime();
    }
}
