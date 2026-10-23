//! The phone keyboard (`interaction.md` section 9): its panel's state and
//! the paired phone's keys, which act as the window's own.

use interface_core::browser::{Key as BrowserKey, Modifiers};

use super::App;

impl App {
    /// The phone keyboard's state, for its panel and the strip.
    pub(super) fn show_phone(&mut self, view: protocol::keyboard::PhoneView) {
        use interface_core::browser::{PhoneStatus, PhoneUiView};
        use protocol::keyboard::PhoneState;
        let status = match view.state {
            PhoneState::Preparing => PhoneStatus::Preparing,
            PhoneState::Scan => PhoneStatus::Scan,
            PhoneState::Connected => PhoneStatus::Connected,
            PhoneState::Waiting => PhoneStatus::Waiting,
            PhoneState::PinFailed(left) => PhoneStatus::PinFailed(left),
            PhoneState::Locked => PhoneStatus::Locked,
            PhoneState::Finished => PhoneStatus::Finished,
        };
        self.view.browser.set_phone(PhoneUiView { status, qr: view.qr.map(|qr| (qr.size, qr.bits)), pin: view.pin });
    }

    /// A key of the paired phone: it acts as a key of the window's own
    /// keyboard, with no modifier (REQ-INT-138).
    pub(super) fn phone_key(&mut self, key: protocol::keyboard::PhoneKey) -> bool {
        use protocol::keyboard::PhoneKey;
        let now = self.now();
        let typing = self.view.browser.typing_target().is_some();
        match key {
            PhoneKey::Text(text) if typing => self.view.browser.terminal_text(&text, now),
            PhoneKey::Text(text) => self.view.browser.key(BrowserKey::Text(&text), Modifiers::default(), now),
            PhoneKey::Key { down, name } => {
                let mut characters = name.chars();
                let single = match (characters.next(), characters.next()) {
                    (Some(character), None) => Some(character),
                    _ => None,
                };
                if typing {
                    return crate::keys::phone_parts(down, &name)
                        .is_some_and(|parts| self.view.browser.terminal_key(parts.action, parts.key, parts.mods, parts.text.as_deref(), parts.base, now));
                }
                if !down {
                    return false;
                }
                let browser_key = match name.as_str() {
                    "Enter" => BrowserKey::Enter,
                    "Escape" | "BrowserBack" | "Exit" => BrowserKey::Escape,
                    "Backspace" => BrowserKey::Backspace,
                    "Tab" => BrowserKey::Tab,
                    "ArrowLeft" => BrowserKey::Left,
                    "ArrowRight" => BrowserKey::Right,
                    "ArrowUp" | "ChannelUp" => BrowserKey::Up,
                    "ArrowDown" | "ChannelDown" => BrowserKey::Down,
                    _ if single.is_some() => BrowserKey::Text(&name),
                    _ => return false,
                };
                self.view.browser.key(browser_key, Modifiers::default(), now)
            }
        }
    }
}
