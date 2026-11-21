//! Typing into a page (`interaction.md`): keys, pastes and the browser's
//! own modifier while keys go to a channel, and the typing state the
//! trusted core sends.

use winit::event::ElementState;
use winit::keyboard::Key;

use super::App;

impl App {
    /// Whether `event` is the browser's paste shortcut, the browser's
    /// modifier and V, while text goes to a channel (REQ-7037).
    pub(super) fn is_paste(&self, event: &winit::event::KeyEvent) -> bool {
        use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
        self.view.browser.text_target().is_some()
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

    /// A key event as the trusted core is given it, or none for a key
    /// that types several characters.
    fn key_parts(&self, event: &winit::event::KeyEvent) -> Option<crate::keys::KeyParts> {
        use neoteletext_input::event::Action;
        use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
        let action = match (event.state, event.repeat) {
            (ElementState::Pressed, false) => Action::Press,
            (ElementState::Pressed, true) => Action::Repeat,
            (ElementState::Released, _) => Action::Release,
        };
        let unmodified = event.key_without_modifiers();
        crate::keys::parts(action, &event.logical_key, &unmodified, event.physical_key, event.text.as_deref(), self.modifiers)
    }

    /// Whether a key is the open page's and not the browser's own viewer's
    /// (REQ-7037): keys go to its channel, the browser's modifier is not
    /// held, and the page's table names the key, or asks for text and the
    /// key types. A key that types several characters goes as text, so it
    /// is the page's while the page asks for text.
    pub(super) fn key_is_pages(&self, event: &winit::event::KeyEvent) -> bool {
        if self.browser_modifier() {
            return false;
        }
        let browser = &self.view.browser;
        match self.key_parts(event) {
            Some(parts) => browser.key_target(parts.action, parts.key, parts.mods, parts.text.as_deref(), parts.base).is_some(),
            None => browser.text_target().is_some(),
        }
    }

    /// A key of the open page's ([`App::key_is_pages`]): it goes pressed,
    /// repeating and released, as the trusted core's session asks for
    /// them (`interaction.md` section 7.3); the browser gives its channel
    /// no key that is not the page's. Nothing is drawn for it: the channel
    /// draws what it makes of the key.
    pub(super) fn terminal_key(&mut self, event: &winit::event::KeyEvent) {
        if self.browser_modifier() {
            return;
        }
        match self.key_parts(event) {
            Some(parts) => self.view.browser.terminal_key(parts.action, parts.key, parts.mods, parts.text.as_deref(), parts.base),
            // A key that types several characters: they go as text.
            None => {
                if let (Some(text), ElementState::Pressed, false) = (&event.text, event.state, event.repeat) {
                    self.view.browser.terminal_text(text);
                }
            }
        }
    }

    /// Whether keys go to the open page's channel, as the trusted core says.
    pub(super) fn show_typing(&mut self, typing: &protocol::interaction::Typing, now: u64) {
        let cursor = typing.cursor.map(|cursor| interface_core::browser::CursorView { row: cursor.row, column: cursor.column, flags: cursor.flags });
        let view = interface_core::browser::TypingView {
            channel: typing.channel,
            page: typing.page,
            on: typing.on,
            secure: typing.secure,
            refused: typing.refused,
            cursor,
            round_trip_us: typing.round_trip_us,
            wheel: typing.wheel,
            asks: typing.asks,
            keys: typing.keys,
        };
        self.view.browser.set_typing(view, now);
        self.view.browser.set_asked_keys(asked_keys(&typing.asked));
        self.sync_ime();
    }
}

/// What of the keyboard the open page's table asks for, as the browser
/// keeps it (REQ-7037). The trusted core's reader has checked that a
/// character's code is a Unicode scalar value; one that is not names no
/// key, and is left out.
fn asked_keys(asked: &protocol::rules::KeyAsks) -> interface_core::browser::AskedKeys {
    use interface_core::browser::{AskedKey, KeyNamed};
    use protocol::rules::KeyKind;
    let keys = asked.keys.iter().filter_map(|ask| {
        let key = match ask.key {
            None => KeyNamed::Any,
            Some(key) => match key.kind {
                KeyKind::Character => KeyNamed::Character(char::from_u32(key.code)?),
                KeyKind::Named => KeyNamed::Named(key.code),
                KeyKind::Physical => KeyNamed::Physical(char::from_u32(key.code)?),
            },
        };
        Some(AskedKey { key, mods_mask: ask.mods_mask, mods_value: ask.mods_value })
    });
    interface_core::browser::AskedKeys { text: asked.text, keys: keys.collect() }
}
