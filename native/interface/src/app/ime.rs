//! The window's input method: on while the address is edited or keys go
//! to a channel. And the platform's preference for reduced motion, the
//! other setting of the system's the window follows.

use super::App;

/// macOS's accessibility setting, "Reduce motion", through `dock-menu`.
#[cfg(target_os = "macos")]
fn system_reduced_motion() -> bool {
    dock_menu::reduced_motion()
}

/// Windows's "Animation effects", through `web-opener`.
#[cfg(windows)]
fn system_reduced_motion() -> bool {
    web_opener::reduced_motion()
}

/// The desktop's preference as the broker read it from the desktop portal
/// before starting this process (DEC-2424): the interface
/// reaches no session bus (decision 86), so a change is followed at the
/// browser's next start.
#[cfg(not(any(target_os = "macos", windows)))]
fn system_reduced_motion() -> bool {
    reduced_by_broker(std::env::var_os(confine::broker::REDUCED_MOTION_VARIABLE).as_deref())
}

/// The broker's word, strictly: its one value, and nothing else, is
/// reduced motion.
#[cfg(any(not(any(target_os = "macos", windows)), test))]
fn reduced_by_broker(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|value| value == confine::broker::REDUCED_MOTION_VALUE)
}

impl App {
    /// The window's moves follow the platform's preference for reduced
    /// motion (REQ-754), asked whenever a window opens or gains the focus.
    pub(super) fn follow_reduced_motion(&mut self) {
        let reduced = system_reduced_motion();
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

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::reduced_by_broker;

    /// REQ-754 on Linux: motion is reduced when the broker set its
    /// variable to its one value, and only then; a variable missing or
    /// holding anything else leaves the moves as they were.
    #[test]
    fn the_broker_alone_says_motion_is_reduced() {
        assert!(reduced_by_broker(Some(OsStr::new(confine::broker::REDUCED_MOTION_VALUE))));
        for other in [None, Some(OsStr::new("")), Some(OsStr::new("0")), Some(OsStr::new("true")), Some(OsStr::new("1 "))] {
            assert!(!reduced_by_broker(other), "{other:?}");
        }
    }
}
