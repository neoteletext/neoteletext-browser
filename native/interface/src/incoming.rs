//! What reaches the window's thread: the trusted core's messages as the
//! reader thread hands them over, and the user's choices from the Dock
//! menu and the menu bar.

use std::collections::BTreeSet;

use dock_menu::{Choice, MenuItem};
use interface_core::browser::{StripState, TabInfo};
use interface_core::canvas::PageKey;
use interface_core::channel::Channel;
use interface_core::library::WindowGeometry;

/// The trusted core's views of history and bookmarks.
pub enum Library {
    /// The history panel's rows.
    History(interface_core::library::HistoryView),
    /// The cookies panel.
    Cookies(interface_core::cookies::CookiesView),
    /// Suggestions for the address field.
    Suggestions(interface_core::library::Suggestions),
    /// The active tab's Back or Forward list.
    TabList(interface_core::library::TabList),
    /// The bookmarks manager.
    Bookmarks(interface_core::library::BookmarksView),
    /// The star's popover.
    Star(interface_core::library::StarView),
    /// The bookmarks bar.
    Bar(interface_core::library::BarView),
    /// The open page's controls.
    PageControls(interface_core::controls::PageControls),
    /// Where the remote control is.
    Remote(interface_core::settings::RemotePlace),
}

/// What the reader thread hands to the window's thread.
pub enum Incoming {
    /// The window's tabs, and the index of the active one.
    Tabs(Vec<TabInfo>, usize),
    /// What the trusted strip shows.
    Strip(StripState),
    /// The active tab's channel.
    Channel(Channel),
    /// A page arrived, or its status changed.
    Arrived {
        /// The channel.
        channel: u32,
        /// The page number.
        page: u16,
        /// Its title, cleaned.
        title: String,
    },
    /// A page has a new version.
    Updated {
        /// The channel.
        channel: u32,
        /// The page number.
        page: u16,
    },
    /// A page image.
    Page {
        /// Which page.
        key: PageKey,
        /// Counts up with each new image of the page.
        version: u32,
        /// In `protocol::frame`'s form, read on the window's thread against
        /// the frame of the page it holds.
        image: Vec<u8>,
    },
    /// About hyperlinks.
    #[cfg(feature = "links")]
    Link(protocol::links::Message),
    /// Whether keys go to the open page's channel.
    #[cfg(feature = "interaction")]
    Typing(protocol::interaction::Typing),
    /// The phone keyboard's panel and state.
    #[cfg(feature = "phone-keyboard")]
    Phone(protocol::keyboard::PhoneView),
    /// A key of the paired phone.
    #[cfg(feature = "phone-keyboard")]
    PhoneKey(protocol::keyboard::PhoneKey),
    /// The channels now playing sound, muted or not.
    Sounding(BTreeSet<u32>),
    /// The settings, for the panel of the trusted strip.
    Settings(interface_core::settings::SettingsView),
    /// Updates, for the settings panel.
    Update(interface_core::settings::UpdateView),
    /// New entries for the developer console.
    Console(interface_core::console::ConsoleUpdate),
    /// History, bookmarks and their panels (`history-and-bookmarks.md`).
    Library(Library),
    /// Where the window was when the session was kept.
    Window(WindowGeometry),
    /// The user chose an item of the Dock menu.
    Dock(Choice),
    /// The user chose an item of the menu bar (decision 84).
    Menu(&'static MenuItem),
    /// The messages that follow are for this window (`multiple-windows.md`).
    Target(u32),
    /// Open a window of this number.
    OpenWindow {
        /// Its number.
        window: u32,
        /// Its place and size, when kept.
        geometry: Option<WindowGeometry>,
    },
    /// Close the window of this number.
    CloseWindow(u32),
    /// The trusted core closed the pipe, asked to close, or broke the protocol.
    End(Option<String>),
}
