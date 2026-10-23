//! The window's input method: on while the address is edited or keys go
//! to a channel.

use super::App;

impl App {
    /// Enables the window's input method while the address is edited or
    /// keys go to a channel, whose composed text then goes to it.
    pub(super) fn sync_ime(&mut self) {
        #[cfg(feature = "interaction")]
        let allowed = self.view.ime.address || self.view.browser.typing_target().is_some();
        #[cfg(not(feature = "interaction"))]
        let allowed = self.view.ime.address;
        if allowed != self.view.ime.allowed
            && let Some(graphics) = &self.view.graphics
        {
            graphics.window.set_ime_allowed(allowed);
            self.view.ime.allowed = allowed;
        }
    }
}
