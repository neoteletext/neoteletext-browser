//! The phone keyboard (`interaction.md` section 9): its panel's state and
//! the paired phone's keys, which type into a page open for typing
//! and otherwise only move between pages (decision 99).

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

    /// A key of the paired phone, with no modifier (REQ-INT-138): while a
    /// page is open for typing it is the page's, as a key of the window's
    /// own keyboard; otherwise it moves between pages and nothing else,
    /// never reaching a field, the strip's question or a panel (decision
    /// 99).
    pub(super) fn phone_key(&mut self, key: protocol::keyboard::PhoneKey) -> bool {
        use protocol::keyboard::PhoneKey;
        let now = self.now();
        let typing = self.view.browser.typing_target().is_some();
        match key {
            PhoneKey::Text(text) if typing => {
                self.view.browser.terminal_text(&text);
                false
            }
            PhoneKey::Text(text) => self.view.browser.phone_text(&text, now),
            PhoneKey::Key { down, name } if typing => {
                if let Some(parts) = crate::keys::phone_parts(down, &name) {
                    self.view.browser.terminal_key(parts.action, parts.key, parts.mods, parts.text.as_deref(), parts.base);
                }
                false
            }
            PhoneKey::Key { down, name } => self.view.browser.phone_key(down, &name, now),
        }
    }
}
