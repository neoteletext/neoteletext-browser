//! The window's input method: on while the address is edited or keys go
//! to a channel. And the platform's preference for reduced motion, the
//! other setting of the system's the window follows.

use super::App;

impl App {
    /// The window's moves follow the platform's preference for reduced
    /// motion (REQ-754), read where the platform offers it: macOS's
    /// accessibility setting through `dock-menu`. Windows and Linux are not
    /// read yet (TODO-2083), and move as they always did.
    pub(super) fn follow_reduced_motion(&mut self) {
        let reduced = dock_menu::reduced_motion();
        if reduced != self.view.browser.wall().reduced_motion() {
            self.view.browser.wall_mut().set_reduced_motion(reduced);
            self.request_redraw();
        }
    }

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
            // A method enabled afresh is told where the caret is again.
            self.view.ime.caret = None;
        }
    }
}
