//! The menu of the browser's Dock icon on macOS (decision 39): each tab,
//! the selected one ticked and those playing sound or muted marked, as a
//! web browser's Dock menu lists its windows, then "Mute All Tabs" or
//! "Unmute All Tabs". Choosing one reports a [`Choice`]. The system adds the
//! window itself to the menu, titled with the selected tab.
//!
//! `AppKit` asks the application's delegate for the menu each time the user
//! opens it (`applicationDockMenu:`). winit owns that delegate, so this
//! crate adds the one method to its class. That needs Objective-C runtime
//! calls, which are `unsafe`: this is the only crate of the browser
//! allowed them. Elsewhere it does nothing.

/// One tab as the Dock menu lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockTab {
    /// The tab's title.
    pub title: String,
    /// The tab is the selected one.
    pub selected: bool,
    /// The tab plays sound, muted or not.
    pub playing: bool,
    /// The user muted the tab.
    pub muted: bool,
}

/// What the user chose in the Dock menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// Select the tab at this index.
    Tab(usize),
    /// Mute every tab, or unmute every tab.
    MuteAll(bool),
}

/// The menu item's title for a tab: marked as the tab strip shows it.
#[must_use]
pub fn tab_title(tab: &DockTab) -> String {
    let mark = if tab.muted {
        "\u{1F507} "
    } else if tab.playing {
        "\u{1F50A} "
    } else {
        ""
    };
    format!("{mark}{}", tab.title)
}

/// The last item's title, and the choice it makes.
#[must_use]
pub fn mute_all_item(tabs: &[DockTab]) -> (&'static str, Choice) {
    if !tabs.is_empty() && tabs.iter().all(|tab| tab.muted) { ("Unmute All Tabs", Choice::MuteAll(false)) } else { ("Mute All Tabs", Choice::MuteAll(true)) }
}

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::{install, set};

/// Elsewhere there is no Dock.
#[cfg(not(target_os = "macos"))]
pub fn install(_choose: impl Fn(Choice) + 'static) {}

/// Elsewhere there is no Dock.
#[cfg(not(target_os = "macos"))]
pub fn set(_tabs: Vec<DockTab>) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(title: &str, playing: bool, muted: bool) -> DockTab {
        DockTab { title: title.into(), selected: false, playing, muted }
    }

    #[test]
    fn a_tab_is_marked_as_its_sound_is() {
        assert_eq!(tab_title(&tab("news", false, false)), "news");
        assert_eq!(tab_title(&tab("news", true, false)), "\u{1F50A} news");
        assert_eq!(tab_title(&tab("news", true, true)), "\u{1F507} news");
        assert_eq!(tab_title(&tab("news", false, true)), "\u{1F507} news");
    }

    #[test]
    fn mute_all_unmutes_once_every_tab_is_muted() {
        assert_eq!(mute_all_item(&[]), ("Mute All Tabs", Choice::MuteAll(true)));
        assert_eq!(mute_all_item(&[tab("a", true, true), tab("b", false, false)]), ("Mute All Tabs", Choice::MuteAll(true)));
        assert_eq!(mute_all_item(&[tab("a", true, true), tab("b", false, true)]), ("Unmute All Tabs", Choice::MuteAll(false)));
    }
}
