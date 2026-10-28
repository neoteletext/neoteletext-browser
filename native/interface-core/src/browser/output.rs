//! What the user did, for the host to send on or act on: the browser's
//! outputs (design section 5.2), and the actions the window itself asks
//! its host for.

use alloc::string::String;
use alloc::vec::Vec;

use crate::canvas::PageKey;
use crate::wall::CellClick;

/// Something the window itself should do, for the host to carry out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowAction {
    /// Move the window: a press on the empty tab strip.
    Drag,
    /// Minimise the window.
    Minimise,
    /// Maximise the window, or restore it.
    Maximise,
    /// Close the window.
    Close,
}

/// What the user did, for the host to send on or act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    /// The pages now on screen (requirement 9.5).
    Visible(Vec<PageKey>),
    /// A click on a cell of the zoomed page.
    Cell(CellClick),
    /// Switch to the tab at this index.
    SelectTab(usize),
    /// Close the tab at this index.
    CloseTab(usize),
    /// Open a new tab.
    NewTab,
    /// Strip input: an address typed into the trusted strip.
    Navigate(String),
    /// Go back in the tab's history.
    Back,
    /// Go forward in the tab's history.
    Forward,
    /// Bookmark the channel, or remove its bookmark.
    Bookmark,
    /// Strip input: open the home address (requirement 19.6).
    Home,
    /// Strip input: open the tab's channel afresh (requirement 11.2).
    Reload,
    /// Strip input: a key of the open page's controls (decision 67).
    PageControl(crate::controls::PageControl),
    /// The sub-page list asks for the rows of `page` from place `first`,
    /// `count` of them, at most [`crate::controls::SUBPAGE_ROWS`]: those
    /// it scrolled into view (decision 112 item 3).
    SubPageRows {
        /// The page.
        page: u16,
        /// The first row's place, from 0.
        first: u16,
        /// How many.
        count: u16,
    },
    /// Strip input: move a tab from one place to another (decision 69).
    MoveTab {
        /// Its place now.
        from: usize,
        /// Its place after.
        to: usize,
    },
    /// Strip input: what the history panel asks for (requirement 20.4).
    History(crate::library::HistoryRequest),
    /// Strip input: what the cookies panel asks for (requirement 21.6).
    Cookies(crate::cookies::CookiesRequest),
    /// Strip input: what the bookmarks manager asks for (requirement 20.17).
    Bookmarks(crate::library::BookmarkRequest),
    /// Strip input: what the star's popover asks for (requirement 20.16).
    Star(crate::library::StarRequest),
    /// Strip input: what the bookmarks bar asks for (requirement 20.18).
    Bar(crate::library::BarRequest),
    /// The address field's text, for suggestions (requirement 20.11).
    Suggest(String),
    /// Strip input: open a suggestion, by its number.
    Suggestion {
        /// The suggestion's number.
        number: u32,
        /// In a new tab.
        new_tab: bool,
    },
    /// Back's or Forward's list, asked for (requirement 20.10).
    TabList(bool),
    /// Strip input: go this many steps back or forward.
    GoTo {
        /// Forward, or back.
        forward: bool,
        /// How many steps.
        steps: u8,
    },
    /// Strip input: open a private tab (requirement 20.8).
    NewPrivateTab,
    /// Strip input: open a new window, private or not
    /// (`multiple-windows.md`).
    NewWindow {
        /// A private window.
        private: bool,
    },
    /// Strip input: a tab dragged off the strip and let go at this point
    /// of the window, in whole logical pixels, which may be outside it:
    /// into the window there, or a new one.
    TabOut {
        /// The tab's place.
        index: usize,
        /// Where it was let go, right of the window's left edge.
        x: i32,
        /// And below its top.
        y: i32,
    },
    /// Strip input: move the active tab into a new window.
    TabToNewWindow,
    /// Strip input: a change the settings panel asks for (requirement 19.3).
    Setting(crate::settings::SettingChange),
    /// Strip input: install the version the settings panel offers
    /// (`installation-and-updates.md` section 3.4).
    InstallUpdate,
    /// The developer console opened or closed (requirement 19.8).
    Console(bool),
    /// Mute the tab at this index, or unmute it.
    MuteTab(usize),
    /// Mute every tab, or unmute every tab.
    MuteAll(bool),
    /// Something the window should do.
    Window(WindowAction),
    /// Whether text input, including input methods, goes to the address field.
    TextInput(bool),
    /// The pointer or the keyboard's focus is on this cell of a link, or on
    /// no link: the trusted strip shows where it leads.
    #[cfg(feature = "links")]
    Point(Option<crate::links::LinkCell>),
    /// The control or link under the pointer, and whether a button is
    /// held on it, or none: its look, which the trusted core has the
    /// decoder draw (INPUT.md section 9.7).
    #[cfg(feature = "links")]
    Lit(Option<crate::links::Lit>),
    /// A page number typed that the open page or the channel links
    /// elsewhere.
    #[cfg(feature = "links")]
    Typed {
        /// The number.
        number: u16,
        /// The open page and its image's version, if a page is open.
        opened: Option<(PageKey, u32)>,
    },
    /// Strip input: yes to the strip's question of this number.
    #[cfg(feature = "links")]
    Confirm(u32),
    /// Strip input: no to the strip's question of this number.
    #[cfg(feature = "links")]
    Cancel(u32),
    /// Input on the open page, whose channel takes it (REQ-INT-132): an
    /// intent, which the trusted core encodes.
    #[cfg(feature = "interaction")]
    Terminal {
        /// The open page.
        key: PageKey,
        /// What the viewer did.
        intent: crate::input::Intent,
    },
    /// Strip input: the phone keyboard's panel opened, or closed.
    #[cfg(feature = "phone-keyboard")]
    PhonePanel(bool),
    /// Strip input: end the phone keyboard's session.
    #[cfg(feature = "phone-keyboard")]
    PhoneFinish,
}
