//! The pipe between the trusted core and the interface
//! (`architecture.md` section 6.4): what the trusted core tells the
//! interface, and the interface's intents. Each side accepts only the
//! kinds meant for it, so the interface can never send what only the
//! trusted core may say.

mod kinds;
mod pages;
mod strip;
mod tabs;
#[cfg(test)]
mod tests;
mod to_core;
mod to_interface;
mod windows;

use interface_core::browser::{StripState, TabInfo};
use interface_core::canvas::PageKey;
use interface_core::channel::Channel;

#[cfg(feature = "interaction")]
use crate::interaction;
#[cfg(feature = "phone-keyboard")]
use crate::keyboard;
#[cfg(feature = "links")]
use crate::links;
use crate::{console, cookies, library, settings};

pub use strip::{problem_code, problem_of};
pub(crate) use tabs::{push_channels, read_channels};
pub use to_core::{read_from_interface, write_to_core};
pub use to_interface::{read_from_core, write_to_interface};

/// From the trusted core to the interface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToInterface {
    /// The window's tabs and which is active.
    Tabs {
        /// Every tab, left to right.
        tabs: Vec<TabInfo>,
        /// The index of the active tab.
        active: usize,
    },
    /// What the trusted strip shows, as the trusted core formatted it.
    Strip(StripState),
    /// The active tab's channel: its banks and every page's status.
    Channel(Channel),
    /// One page's new status.
    Page {
        /// The channel.
        channel: u32,
        /// The page number.
        page: u16,
        /// Its title, cleaned.
        title: String,
    },
    /// A page's sub-pages, each with its title, cleaned, in ascending order;
    /// none when it has one again: the page list lists them.
    SubPages {
        /// The channel.
        channel: u32,
        /// The page number.
        page: u16,
        /// Each sub-page's sub-code and title.
        subpages: Vec<(u16, String)>,
    },
    /// A page has a new version: the interface shows that it changed.
    Updated {
        /// The channel.
        channel: u32,
        /// The page number.
        page: u16,
    },
    /// One page image, checked and encoded by the trusted core.
    Frame {
        /// Which page.
        key: PageKey,
        /// Counts up with each new image of the page, so an older one never replaces a newer.
        version: u32,
        /// The image, in [`frame`](crate::frame)'s form.
        image: Vec<u8>,
    },
    /// The channels sounding now, muted or not, for the tabs' speakers;
    /// the sound helper says it and the trusted core passes it on
    /// (decision 55).
    Sounding(Vec<u32>),
    /// About hyperlinks (`hyperlinks.md` section 8.1).
    #[cfg(feature = "links")]
    Link(links::Message),
    /// Whether keys go to the open page's channel, and its cursor
    /// (`interaction.md` section 11).
    #[cfg(feature = "interaction")]
    Typing(interaction::Typing),
    /// The phone keyboard's panel and state (INPUT.md section 10).
    #[cfg(feature = "phone-keyboard")]
    Phone(keyboard::PhoneView),
    /// A key of the paired phone, which the interface takes as its own.
    #[cfg(feature = "phone-keyboard")]
    PhoneKey(keyboard::PhoneKey),
    /// The settings, for the panel of the trusted strip (requirement
    /// 19.3) and the mixer's sound and volume (requirement 19.4).
    Settings(settings::SettingsView),
    /// Updates, for the settings panel (`installation-and-updates.md`
    /// section 3.4).
    Update(settings::UpdateView),
    /// New entries for the developer console (requirement 19.9).
    Console(console::ConsoleUpdate),
    /// The history panel's rows (requirement 20.4).
    History(library::HistoryView),
    /// Suggestions for what is typed in the address field (requirement 20.11).
    Suggestions(library::Suggestions),
    /// The active tab's Back or Forward list (requirement 20.10).
    TabList(library::TabList),
    /// The bookmarks manager's folders and items (requirement 20.17).
    Bookmarks(library::BookmarksView),
    /// The star's popover (requirement 20.16).
    Star(library::StarView),
    /// The bookmarks bar (requirement 20.18).
    Bar(library::BarView),
    /// The window's place and size, restored from the last session
    /// (requirement 20.22).
    Window(library::WindowGeometry),
    /// The cookies panel (requirement 21.6).
    Cookies(cookies::CookiesView),
    /// The open page's controls (`page-controls.md`, decision 67).
    PageControls(interface_core::controls::PageControls),
    /// Where the remote control is, as kept (`remote-control.md`, decision
    /// 68).
    Remote(settings::RemotePlace),
    /// The messages that follow are for this window, until the next
    /// (`multiple-windows.md`); before the first, for the first window.
    Target(u32),
    /// Open a window of this number, at this place if kept.
    OpenWindow {
        /// Its number, never [`NEW_WINDOW`](crate::NEW_WINDOW).
        window: u32,
        /// Its place and size, when kept.
        geometry: Option<library::WindowGeometry>,
    },
    /// Close the window of this number: its tabs are gone.
    CloseWindow(u32),
    /// The browser is closing: the interface ends.
    Close,
}

/// From the interface to the trusted core. Intents cannot name another
/// channel's address or change what is fetched (requirements 4.5 and 4.7);
/// only strip input, a separate set of message kinds, can navigate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToCore {
    /// The pages now on screen (requirement 9.5).
    Visible(Vec<PageKey>),
    /// A click on one character cell of a page (requirement 12.3).
    Cell {
        /// Which page.
        key: PageKey,
        /// The image of the page that was on screen.
        version: u32,
        /// The cell's column, 0 to 39.
        column: u8,
        /// The cell's row, 0 to 24.
        row: u8,
    },
    /// Switch to the tab at this index.
    SelectTab(u8),
    /// Close the tab at this index.
    CloseTab(u8),
    /// Strip input: move the tab at the first place to the second, as a
    /// drag along the strip asks (`tab-strip.md`, decision 69).
    MoveTab {
        /// Its place now.
        from: u8,
        /// Its place after.
        to: u8,
    },
    /// Open a new tab.
    NewTab,
    /// The page the user opened, zoomed on the wall, or none: its sounds
    /// play while it is open (the engine's REQ-AUD-012).
    Opened(Option<PageKey>),
    /// Mute the tab at this index, or unmute it.
    MuteTab(u8),
    /// Mute every tab, or unmute every tab.
    MuteAll(bool),
    /// The cell under the pointer or the keyboard's focus, when it is on a
    /// link, or none: the trusted strip shows where it leads.
    #[cfg(feature = "links")]
    Point(Option<links::Spot>),
    /// A page number typed that the open page or the channel links
    /// elsewhere.
    #[cfg(feature = "links")]
    Typed {
        /// The number.
        number: u16,
        /// The open page and its image's version, if a page is open.
        opened: Option<(PageKey, u32)>,
    },
    /// The window gained or lost the keyboard focus.
    #[cfg(any(feature = "links", feature = "interaction"))]
    Focused(bool),
    /// The viewer began or stopped editing the address: while they do,
    /// keys are not the open page's, and the channel has `FOCUS_OUT`.
    #[cfg(feature = "interaction")]
    Editing(bool),
    /// A key, text, the mouse, a paste or a composition on the open page,
    /// for its channel if it takes input (REQ-INT-130, REQ-INT-132).
    #[cfg(feature = "interaction")]
    Input(interaction::InputIntent),
    /// Strip input: open the phone keyboard's panel, or close it.
    #[cfg(feature = "phone-keyboard")]
    PhonePanel(bool),
    /// Strip input: end the phone keyboard's session.
    #[cfg(feature = "phone-keyboard")]
    PhoneFinish,
    /// Strip input: an address typed into the trusted strip.
    Navigate(String),
    /// Strip input: go back in the tab's history.
    Back,
    /// Strip input: go forward in the tab's history.
    Forward,
    /// Strip input: bookmark the channel, or remove its bookmark.
    Bookmark,
    /// Strip input: open the home address in the active tab (requirement
    /// 19.6).
    Home,
    /// Strip input: open the active tab's channel afresh, where it is in its
    /// history (requirement 11.2).
    Reload,
    /// Strip input: a change the settings panel asks for (requirement
    /// 19.3).
    ChangeSetting(settings::SettingChange),
    /// The window drew its first frame: a part of the version's health
    /// (`installation-and-updates.md` section 3.2).
    Drawn,
    /// Strip input: install the version the settings panel offers
    /// (section 3.4).
    InstallUpdate,
    /// The developer console opened or closed (requirement 19.8).
    Console(bool),
    /// Strip input: yes to the strip's question of this number.
    #[cfg(feature = "links")]
    Confirm(u32),
    /// Strip input: no to the strip's question of this number.
    #[cfg(feature = "links")]
    Cancel(u32),
    /// Strip input: what the history panel asks for (requirements 20.4 to
    /// 20.9).
    History(library::HistoryRequest),
    /// Strip input: what the bookmarks manager asks for (requirement 20.17).
    Bookmarks(library::BookmarkRequest),
    /// Strip input: what the star's popover asks for (requirement 20.16).
    Star(library::StarRequest),
    /// Strip input: what the bookmarks bar asks for (requirement 20.18).
    Bar(library::BarRequest),
    /// The address field's text, for suggestions (requirement 20.11).
    Suggest(String),
    /// Strip input: open a suggestion, by its number.
    Suggestion {
        /// The suggestion's number.
        number: u32,
        /// In a new tab.
        new_tab: bool,
    },
    /// The active tab's Back or Forward list, asked for (requirement 20.10).
    TabList(bool),
    /// Strip input: move the active tab this many steps back or forward.
    GoTo {
        /// Forward, or back.
        forward: bool,
        /// How many steps, 1 to `library::MAX_STEPS`.
        steps: u8,
    },
    /// Strip input: open a private tab (requirement 20.8).
    NewPrivateTab,
    /// The window moved, was resized, or was maximised or restored, for
    /// the session (requirement 20.21).
    Window(library::WindowGeometry),
    /// Strip input from the cookies panel (requirement 21.6).
    Cookies(cookies::CookiesRequest),
    /// Strip input: a key of the open page's controls (decision 67).
    PageControl(interface_core::controls::PageControl),
    /// The intents that follow are from this window, until the next
    /// (`multiple-windows.md`).
    From(u32),
    /// Strip input: open a new window, private or not.
    NewWindow {
        /// A private window.
        private: bool,
    },
    /// Strip input: move the tab at this place of the window the intent is
    /// from into another window, before the tab at `index` there; to a new
    /// window when `window` is [`NEW_WINDOW`](crate::NEW_WINDOW).
    TabToWindow {
        /// Its place now.
        tab: u8,
        /// The window it moves to.
        window: u32,
        /// Its place there.
        index: u8,
    },
    /// Strip input: close the window the intent is from, and its tabs.
    CloseWindow,
}
