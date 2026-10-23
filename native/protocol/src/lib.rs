//! Messages between the trusted core and the interface process
//! (`architecture.md` section 6.4). The framing is the engine's: one kind
//! byte, a 32-bit big-endian payload length, then the payload. Every length is bounded
//! before anything is allocated, every payload is parsed strictly to its
//! exact length, and each side accepts only the kinds meant for it, so the
//! interface can never send what only the trusted core may say.

// No panicking constructs: this crate reads input from outside the
// process. The same list is in each such crate; scripts/check.sh checks
// that they agree.
#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

pub mod audio;
mod bounds;
pub mod console;
mod controls;
pub mod cookies;
pub mod decoder;
mod error;
pub mod fetcher;
mod fields;
pub mod frame;
mod framing;
#[cfg(feature = "interaction")]
pub mod interaction;
#[cfg(feature = "phone-keyboard")]
pub mod keyboard;
pub mod library;
#[cfg(feature = "links")]
pub mod links;
pub mod settings;
pub mod sound;
mod text;

pub use bounds::*;
pub use error::ProtocolError;
pub(crate) use fields::{Fields, KEY_BYTES, push_count, push_count_u8, push_flag, push_key, push_keys, push_text};
pub(crate) use framing::{read_frame, tail, write_frame};
pub use text::clean;
pub(crate) use text::{push_raw_text, raw_text};

use std::io::{Read, Write};

use interface_core::browser::{CertificateProblem, StripState, TabInfo};
use interface_core::canvas::{COLUMNS, PageKey, ROWS};
use interface_core::channel::{BANKS, Channel, PAGES, PageInfo, Status, page_index};
use interface_core::theme::TAB_COLOURS;

// Trusted core to interface.
const KIND_STRIP: u8 = 2;
const KIND_FRAME: u8 = 3;
const KIND_CLOSE: u8 = 4;
const KIND_TABS: u8 = 5;
const KIND_CHANNEL: u8 = 6;
const KIND_PAGE: u8 = 8;
const KIND_UPDATED: u8 = 9;
const KIND_TAB_SOUNDING: u8 = 10;
#[cfg(feature = "interaction")]
const KIND_TYPING: u8 = 40;
#[cfg(feature = "phone-keyboard")]
const KIND_PHONE: u8 = 41;
#[cfg(feature = "phone-keyboard")]
const KIND_PHONE_KEY: u8 = 42;
const KIND_SETTINGS: u8 = 43;
const KIND_CONSOLE_UPDATE: u8 = 44;
// Windows (`multiple-windows.md`, decision 72).
const KIND_TARGET: u8 = 46;
const KIND_OPEN_WINDOW: u8 = 47;
const KIND_CLOSE_WINDOW_VIEW: u8 = 49;
// Interface to trusted core: intents.
const KIND_VISIBLE: u8 = 16;
const KIND_CELL: u8 = 17;
const KIND_SELECT_TAB: u8 = 18;
const KIND_CLOSE_TAB: u8 = 19;
const KIND_NEW_TAB: u8 = 20;
const KIND_OPENED: u8 = 21;
const KIND_MUTE_TAB: u8 = 22;
const KIND_MUTE_ALL: u8 = 23;
#[cfg(feature = "links")]
const KIND_POINT: u8 = 24;
#[cfg(feature = "links")]
const KIND_TYPED: u8 = 25;
#[cfg(any(feature = "links", feature = "interaction"))]
const KIND_FOCUSED: u8 = 26;
#[cfg(feature = "interaction")]
const KIND_INPUT: u8 = 27;
#[cfg(feature = "phone-keyboard")]
const KIND_PHONE_PANEL: u8 = 28;
#[cfg(feature = "phone-keyboard")]
const KIND_PHONE_FINISH: u8 = 29;
#[cfg(feature = "interaction")]
const KIND_EDITING: u8 = 30;
// Interface to trusted core: strip input, kinds of their own
// (`architecture.md` section 6.4).
const KIND_NAVIGATE: u8 = 32;
const KIND_BACK: u8 = 33;
const KIND_FORWARD: u8 = 34;
const KIND_BOOKMARK: u8 = 35;
#[cfg(feature = "links")]
const KIND_CONFIRM: u8 = 36;
#[cfg(feature = "links")]
const KIND_CANCEL: u8 = 37;
const KIND_CONSOLE: u8 = 38;
const KIND_HOME: u8 = 39;
const KIND_CHANGE_SETTING: u8 = 45;
// History and bookmarks (`history-and-bookmarks.md` section 5): the
// trusted core's views, then the interface's requests.
const KIND_HISTORY_VIEW: u8 = 50;
const KIND_SUGGESTIONS: u8 = 51;
const KIND_TAB_LIST_VIEW: u8 = 52;
const KIND_BOOKMARKS_VIEW: u8 = 53;
const KIND_STAR_VIEW: u8 = 54;
const KIND_BAR_VIEW: u8 = 55;
const KIND_WINDOW_VIEW: u8 = 56;
// The cookies panel (`cookies.md` section 9).
const KIND_COOKIES_VIEW: u8 = 57;
const KIND_PAGE_CONTROLS: u8 = 58;
const KIND_REMOTE: u8 = 59;
const KIND_HISTORY: u8 = 60;
const KIND_BOOKMARKS: u8 = 61;
const KIND_STAR: u8 = 62;
const KIND_BAR: u8 = 63;
const KIND_SUGGEST: u8 = 64;
const KIND_SUGGESTION: u8 = 65;
const KIND_TAB_LIST: u8 = 66;
const KIND_GO_TO: u8 = 67;
const KIND_NEW_PRIVATE_TAB: u8 = 68;
const KIND_WINDOW: u8 = 69;
const KIND_COOKIES: u8 = 70;
const KIND_RELOAD: u8 = 71;
const KIND_PAGE_CONTROL: u8 = 72;
const KIND_MOVE_TAB: u8 = 73;
const KIND_FROM: u8 = 74;
const KIND_NEW_WINDOW: u8 = 75;
const KIND_TAB_TO_WINDOW: u8 = 76;
const KIND_CLOSE_WINDOW: u8 = 77;
// Updates (`installation-and-updates.md` sections 3.2 and 3.4), far from
// the others, so kinds added beside these elsewhere do not meet them.
const KIND_UPDATE_VIEW: u8 = 120;
const KIND_DRAWN: u8 = 121;
const KIND_INSTALL_UPDATE: u8 = 122;

// The strip's flags.
const ENCRYPTED: u8 = 1;
const CAN_GO_BACK: u8 = 2;
const CAN_GO_FORWARD: u8 = 4;
const BOOKMARKED: u8 = 8;
const LOCAL: u8 = 16;
const BUILT_IN: u8 = 32;
const PRIVATE: u8 = 64;
const ALL_FLAGS: u8 = ENCRYPTED | CAN_GO_BACK | CAN_GO_FORWARD | BOOKMARKED | LOCAL | BUILT_IN | PRIVATE;
// A tab's flags.
const TAB_MUTED: u8 = 1;
const TAB_PRIVATE: u8 = 2;
const TAB_FLAGS: u8 = TAB_MUTED | TAB_PRIVATE;

const FRAME_FIXED_BYTES: usize = KEY_BYTES + 4;
const CELL_BYTES: usize = KEY_BYTES + 4 + 2;

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
        /// The image, in [`frame`]'s form.
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
        /// Its number, never [`NEW_WINDOW`].
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
    /// window when `window` is [`NEW_WINDOW`].
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

/// One of the intents of updates (`installation-and-updates.md` section
/// 3.2), which carry nothing.
fn update_intent(kind: u8) -> ToCore {
    if kind == KIND_DRAWN { ToCore::Drawn } else { ToCore::InstallUpdate }
}

/// One of the window intents (`multiple-windows.md`).
fn window_intent(kind: u8, fields: &mut Fields<'_>) -> Result<ToCore, ProtocolError> {
    Ok(match kind {
        KIND_FROM => ToCore::From(window_number(fields)?),
        KIND_NEW_WINDOW => ToCore::NewWindow { private: fields.flag("flag")? },
        KIND_TAB_TO_WINDOW => {
            let tab = fields.u8()?;
            let window = fields.u32()?;
            let index = fields.u8()?;
            if usize::from(tab) >= MAX_TABS || usize::from(index) > MAX_TABS {
                return Err(ProtocolError::BadValue("tab place"));
            }
            ToCore::TabToWindow { tab, window, index }
        }
        KIND_CLOSE_WINDOW => ToCore::CloseWindow,
        other => return Err(ProtocolError::UnknownKind(other)),
    })
}

/// A certificate problem's code on the wire: its place among
/// [`CertificateProblem::ALL`], from 1; 0 is none.
#[must_use]
pub fn problem_code(problem: CertificateProblem) -> u8 {
    let place = CertificateProblem::ALL.iter().position(|each| *each == problem).unwrap_or(0);
    u8::try_from(place).map_or(u8::MAX, |place| place.saturating_add(1))
}

/// The certificate problem of a code, when it names one.
#[must_use]
pub fn problem_of(code: u8) -> Option<CertificateProblem> {
    CertificateProblem::ALL.get(usize::from(code).checked_sub(1)?).copied()
}

/// A window's number, never [`NEW_WINDOW`].
fn window_number(fields: &mut Fields<'_>) -> Result<u32, ProtocolError> {
    let window = fields.u32()?;
    if window == NEW_WINDOW { Err(ProtocolError::BadValue("window number")) } else { Ok(window) }
}

/// Writes a message to the interface.
///
/// # Errors
/// When writing fails, or the message breaks a bound or carries text that
/// was not cleaned.
#[allow(clippy::too_many_lines, reason = "one arm for each kind of message, in the order of the kinds")]
pub fn write_to_interface(out: &mut dyn Write, message: &ToInterface) -> Result<(), ProtocolError> {
    let mut payload = Vec::new();
    let kind = match message {
        ToInterface::Tabs { tabs, active } => {
            if tabs.len() > MAX_TABS || (*active >= tabs.len() && !tabs.is_empty()) {
                return Err(ProtocolError::BadValue("tabs"));
            }
            payload.push(u8::try_from(*active).map_err(|_| ProtocolError::BadValue("tabs"))?);
            payload.push(u8::try_from(tabs.len()).map_err(|_| ProtocolError::BadValue("tabs"))?);
            for tab in tabs {
                payload.push(tab.colour);
                payload.extend_from_slice(&tab.channel.to_be_bytes());
                payload.push(if tab.muted { TAB_MUTED } else { 0 } | if tab.private { TAB_PRIVATE } else { 0 });
                push_text(&mut payload, &tab.title, MAX_TAB_TITLE)?;
            }
            KIND_TABS
        }
        ToInterface::Strip(strip) => {
            let flag = |set: bool, bit: u8| if set { bit } else { 0 };
            payload.push(
                flag(strip.encrypted, ENCRYPTED)
                    | flag(strip.back, CAN_GO_BACK)
                    | flag(strip.forward, CAN_GO_FORWARD)
                    | flag(strip.bookmarked, BOOKMARKED)
                    | flag(strip.local, LOCAL)
                    | flag(strip.built_in, BUILT_IN)
                    | flag(strip.private, PRIVATE),
            );
            push_text(&mut payload, &strip.identity, MAX_IDENTITY)?;
            push_text(&mut payload, &strip.address, MAX_ADDRESS)?;
            push_text(&mut payload, &strip.notice, MAX_STRIP_NOTICE)?;
            payload.push(strip.refused.map_or(0, problem_code));
            KIND_STRIP
        }
        ToInterface::Channel(channel) => {
            if channel.banks.len() != BANKS || channel.pages.len() != PAGES {
                return Err(ProtocolError::BadValue("channel shape"));
            }
            payload.extend_from_slice(&channel.channel.to_be_bytes());
            push_text(&mut payload, &channel.title, MAX_TITLE)?;
            for name in &channel.banks {
                push_text(&mut payload, name, MAX_BANK_NAME)?;
            }
            let present = channel.pages.iter().flatten().count();
            payload.extend_from_slice(&u16::try_from(present).map_err(|_| ProtocolError::BadValue("pages"))?.to_be_bytes());
            for (index, page) in channel.pages.iter().enumerate() {
                let Some(page) = page else { continue };
                let number = interface_core::channel::page_number(index).ok_or(ProtocolError::BadValue("pages"))?;
                payload.extend_from_slice(&number.to_be_bytes());
                payload.push(page.status as u8);
                push_text(&mut payload, &page.title, MAX_PAGE_TITLE)?;
            }
            KIND_CHANNEL
        }
        ToInterface::Page { channel, page, title } => {
            payload.extend_from_slice(&channel.to_be_bytes());
            payload.extend_from_slice(&page.to_be_bytes());
            push_text(&mut payload, title, MAX_PAGE_TITLE)?;
            KIND_PAGE
        }
        ToInterface::Updated { channel, page } => {
            payload.extend_from_slice(&channel.to_be_bytes());
            payload.extend_from_slice(&page.to_be_bytes());
            KIND_UPDATED
        }
        ToInterface::Frame { key, version, image } => {
            payload.reserve_exact(FRAME_FIXED_BYTES.saturating_add(image.len()));
            push_key(&mut payload, *key);
            payload.extend_from_slice(&version.to_be_bytes());
            payload.extend_from_slice(image);
            KIND_FRAME
        }
        ToInterface::Sounding(channels) => {
            if channels.len() > MAX_TABS {
                return Err(ProtocolError::BadValue("too many channels"));
            }
            for channel in channels {
                payload.extend_from_slice(&channel.to_be_bytes());
            }
            KIND_TAB_SOUNDING
        }
        #[cfg(feature = "links")]
        ToInterface::Link(message) => links::push_message(&mut payload, message)?,
        #[cfg(feature = "interaction")]
        ToInterface::Typing(typing) => {
            interaction::push_typing(&mut payload, typing)?;
            KIND_TYPING
        }
        #[cfg(feature = "phone-keyboard")]
        ToInterface::Phone(view) => {
            keyboard::push_view(&mut payload, view)?;
            KIND_PHONE
        }
        #[cfg(feature = "phone-keyboard")]
        ToInterface::PhoneKey(key) => {
            keyboard::push_phone_key(&mut payload, key)?;
            KIND_PHONE_KEY
        }
        ToInterface::Settings(view) => {
            settings::push_view(&mut payload, view)?;
            KIND_SETTINGS
        }
        ToInterface::Update(view) => {
            settings::push_update(&mut payload, view)?;
            KIND_UPDATE_VIEW
        }
        ToInterface::Console(update) => {
            console::push_update(&mut payload, update)?;
            KIND_CONSOLE_UPDATE
        }
        ToInterface::History(view) => {
            library::push_history_view(&mut payload, view)?;
            KIND_HISTORY_VIEW
        }
        ToInterface::Suggestions(suggestions) => {
            library::push_suggestions(&mut payload, suggestions)?;
            KIND_SUGGESTIONS
        }
        ToInterface::TabList(list) => {
            library::push_tab_list(&mut payload, list)?;
            KIND_TAB_LIST_VIEW
        }
        ToInterface::Bookmarks(view) => {
            library::push_bookmarks_view(&mut payload, view)?;
            KIND_BOOKMARKS_VIEW
        }
        ToInterface::Star(view) => {
            library::push_star_view(&mut payload, view)?;
            KIND_STAR_VIEW
        }
        ToInterface::Bar(view) => {
            library::push_bar_view(&mut payload, view)?;
            KIND_BAR_VIEW
        }
        ToInterface::Window(window) => {
            library::push_window(&mut payload, window)?;
            KIND_WINDOW_VIEW
        }
        ToInterface::Cookies(view) => {
            cookies::push_view(&mut payload, view)?;
            KIND_COOKIES_VIEW
        }
        ToInterface::PageControls(view) => {
            controls::push_controls(&mut payload, view)?;
            KIND_PAGE_CONTROLS
        }
        ToInterface::Remote(place) => {
            settings::push_remote(&mut payload, *place)?;
            KIND_REMOTE
        }
        ToInterface::Target(window) => {
            payload.extend_from_slice(&window.to_be_bytes());
            KIND_TARGET
        }
        ToInterface::OpenWindow { window, geometry } => {
            if *window == NEW_WINDOW {
                return Err(ProtocolError::BadValue("window number"));
            }
            payload.extend_from_slice(&window.to_be_bytes());
            if let Some(geometry) = geometry {
                library::push_window(&mut payload, geometry)?;
            }
            KIND_OPEN_WINDOW
        }
        ToInterface::CloseWindow(window) => {
            payload.extend_from_slice(&window.to_be_bytes());
            KIND_CLOSE_WINDOW_VIEW
        }
        ToInterface::Close => KIND_CLOSE,
    };
    write_frame(out, kind, &payload)
}

/// Reads one message from the trusted core, or `None` at a clean end.
///
/// # Errors
/// On input errors, a kind the trusted core may not send, and any payload
/// that is malformed or out of bounds.
#[allow(clippy::too_many_lines, reason = "one arm for each kind of message, in the order of the kinds")]
pub fn read_from_core(input: &mut dyn Read) -> Result<Option<ToInterface>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    let mut fields = Fields::new(&payload);
    let message = match kind {
        KIND_TABS => {
            let active = usize::from(fields.u8()?);
            let count = usize::from(fields.u8()?);
            if count > MAX_TABS || (active >= count && count > 0) {
                return Err(ProtocolError::BadValue("tabs"));
            }
            let tabs = (0..count)
                .map(|_| {
                    let colour = fields.u8()?;
                    if usize::from(colour) >= TAB_COLOURS.len() {
                        return Err(ProtocolError::BadValue("tab colour"));
                    }
                    let channel = fields.u32()?;
                    let flags = fields.u8()?;
                    if flags & !TAB_FLAGS != 0 {
                        return Err(ProtocolError::BadValue("tab flags"));
                    }
                    Ok(TabInfo { colour, channel, muted: flags & TAB_MUTED != 0, private: flags & TAB_PRIVATE != 0, title: fields.text(MAX_TAB_TITLE)? })
                })
                .collect::<Result<_, _>>()?;
            ToInterface::Tabs { tabs, active }
        }
        KIND_STRIP => {
            let flags = fields.u8()?;
            if flags & !ALL_FLAGS != 0 {
                return Err(ProtocolError::BadValue("strip flags"));
            }
            let identity = fields.text(MAX_IDENTITY)?;
            ToInterface::Strip(StripState {
                address: fields.text(MAX_ADDRESS)?,
                identity,
                encrypted: flags & ENCRYPTED != 0,
                back: flags & CAN_GO_BACK != 0,
                forward: flags & CAN_GO_FORWARD != 0,
                bookmarked: flags & BOOKMARKED != 0,
                local: flags & LOCAL != 0,
                built_in: flags & BUILT_IN != 0,
                private: flags & PRIVATE != 0,
                notice: fields.text(MAX_STRIP_NOTICE)?,
                refused: match fields.u8()? {
                    0 => None,
                    code => Some(problem_of(code).ok_or(ProtocolError::BadValue("certificate problem"))?),
                },
            })
        }
        KIND_CHANNEL => {
            let mut channel = Channel::empty();
            channel.channel = fields.u32()?;
            channel.title = fields.text(MAX_TITLE)?;
            for name in &mut channel.banks {
                *name = fields.text(MAX_BANK_NAME)?;
            }
            let count = usize::from(fields.u16()?);
            if count > PAGES {
                return Err(ProtocolError::BadValue("too many pages"));
            }
            for _ in 0..count {
                let index = page_index(fields.u16()?).ok_or(ProtocolError::BadValue("page number"))?;
                let status = Status::from_byte(fields.u8()?).ok_or(ProtocolError::BadValue("page status"))?;
                let title = fields.text(MAX_PAGE_TITLE)?;
                let slot = channel.pages.get_mut(index).ok_or(ProtocolError::BadValue("page number"))?;
                if slot.replace(PageInfo { title, status }).is_some() {
                    return Err(ProtocolError::BadValue("page listed twice"));
                }
            }
            ToInterface::Channel(channel)
        }
        KIND_PAGE => {
            let channel = fields.u32()?;
            let page = fields.display_page()?;
            ToInterface::Page { channel, page, title: fields.text(MAX_PAGE_TITLE)? }
        }
        KIND_UPDATED => {
            let channel = fields.u32()?;
            let page = fields.display_page()?;
            ToInterface::Updated { channel, page }
        }
        KIND_FRAME => {
            if payload.len() < FRAME_FIXED_BYTES {
                return Err(ProtocolError::BadLength { kind, length: payload.len() });
            }
            let key = fields.key()?;
            let version = fields.u32()?;
            let image = tail(payload, FRAME_FIXED_BYTES);
            return Ok(Some(ToInterface::Frame { key, version, image }));
        }
        KIND_TAB_SOUNDING => {
            let mut channels = Vec::new();
            while !fields.is_empty() {
                if channels.len() == MAX_TABS {
                    return Err(ProtocolError::BadValue("too many channels"));
                }
                channels.push(fields.u32()?);
            }
            ToInterface::Sounding(channels)
        }
        #[cfg(feature = "links")]
        kind if links::is_message(kind) => ToInterface::Link(links::read_message(kind, &mut fields)?),
        #[cfg(feature = "interaction")]
        KIND_TYPING => ToInterface::Typing(interaction::read_typing(&mut fields)?),
        #[cfg(feature = "phone-keyboard")]
        KIND_PHONE => ToInterface::Phone(keyboard::read_view(&mut fields)?),
        #[cfg(feature = "phone-keyboard")]
        KIND_PHONE_KEY => ToInterface::PhoneKey(keyboard::read_phone_key(&mut fields)?),
        KIND_SETTINGS => ToInterface::Settings(settings::read_view(&mut fields)?),
        KIND_UPDATE_VIEW => ToInterface::Update(settings::read_update(&mut fields)?),
        KIND_CONSOLE_UPDATE => ToInterface::Console(console::read_update(&mut fields)?),
        KIND_HISTORY_VIEW => ToInterface::History(library::read_history_view(&mut fields)?),
        KIND_SUGGESTIONS => ToInterface::Suggestions(library::read_suggestions(&mut fields)?),
        KIND_TAB_LIST_VIEW => ToInterface::TabList(library::read_tab_list(&mut fields)?),
        KIND_BOOKMARKS_VIEW => ToInterface::Bookmarks(library::read_bookmarks_view(&mut fields)?),
        KIND_STAR_VIEW => ToInterface::Star(library::read_star_view(&mut fields)?),
        KIND_BAR_VIEW => ToInterface::Bar(library::read_bar_view(&mut fields)?),
        KIND_WINDOW_VIEW => ToInterface::Window(library::read_window(&mut fields)?),
        KIND_COOKIES_VIEW => ToInterface::Cookies(cookies::read_view(&mut fields)?),
        KIND_PAGE_CONTROLS => ToInterface::PageControls(controls::read_controls(&mut fields)?),
        KIND_REMOTE => ToInterface::Remote(settings::read_remote(&mut fields)?),
        KIND_TARGET => ToInterface::Target(window_number(&mut fields)?),
        KIND_OPEN_WINDOW => {
            let window = window_number(&mut fields)?;
            let geometry = if fields.is_empty() { None } else { Some(library::read_window(&mut fields)?) };
            ToInterface::OpenWindow { window, geometry }
        }
        KIND_CLOSE_WINDOW_VIEW => ToInterface::CloseWindow(window_number(&mut fields)?),
        KIND_CLOSE => ToInterface::Close,
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    fields.end()?;
    Ok(Some(message))
}

/// Writes a message to the trusted core.
///
/// # Errors
/// When writing fails or the message breaks a bound.
#[allow(clippy::too_many_lines, reason = "one arm for each kind of message, in the order of the kinds")]
pub fn write_to_core(out: &mut dyn Write, message: &ToCore) -> Result<(), ProtocolError> {
    let mut payload = Vec::new();
    let kind = match message {
        ToCore::Visible(keys) => {
            push_keys(&mut payload, keys)?;
            KIND_VISIBLE
        }
        ToCore::Cell { key, version, column, row } => {
            push_key(&mut payload, *key);
            payload.extend_from_slice(&version.to_be_bytes());
            payload.extend_from_slice(&[*column, *row]);
            KIND_CELL
        }
        ToCore::SelectTab(index) => {
            payload.push(*index);
            KIND_SELECT_TAB
        }
        ToCore::CloseTab(index) => {
            payload.push(*index);
            KIND_CLOSE_TAB
        }
        ToCore::MoveTab { from, to } => {
            if usize::from(*from) >= MAX_TABS || usize::from(*to) >= MAX_TABS {
                return Err(ProtocolError::BadValue("tab place"));
            }
            payload.extend_from_slice(&[*from, *to]);
            KIND_MOVE_TAB
        }
        ToCore::NewTab => KIND_NEW_TAB,
        ToCore::Opened(key) => {
            if let Some(key) = key {
                push_key(&mut payload, *key);
            }
            KIND_OPENED
        }
        ToCore::MuteTab(index) => {
            payload.push(*index);
            KIND_MUTE_TAB
        }
        ToCore::MuteAll(muted) => {
            payload.push(u8::from(*muted));
            KIND_MUTE_ALL
        }
        ToCore::Navigate(address) => {
            push_text(&mut payload, address, MAX_ADDRESS)?;
            KIND_NAVIGATE
        }
        ToCore::Back => KIND_BACK,
        ToCore::Forward => KIND_FORWARD,
        ToCore::Bookmark => KIND_BOOKMARK,
        ToCore::Home => KIND_HOME,
        ToCore::Reload => KIND_RELOAD,
        ToCore::PageControl(control) => {
            controls::push_control(&mut payload, *control);
            KIND_PAGE_CONTROL
        }
        ToCore::From(window) => {
            if *window == NEW_WINDOW {
                return Err(ProtocolError::BadValue("window number"));
            }
            payload.extend_from_slice(&window.to_be_bytes());
            KIND_FROM
        }
        ToCore::NewWindow { private } => {
            payload.push(u8::from(*private));
            KIND_NEW_WINDOW
        }
        ToCore::TabToWindow { tab, window, index } => {
            if usize::from(*tab) >= MAX_TABS || usize::from(*index) > MAX_TABS {
                return Err(ProtocolError::BadValue("tab place"));
            }
            payload.push(*tab);
            payload.extend_from_slice(&window.to_be_bytes());
            payload.push(*index);
            KIND_TAB_TO_WINDOW
        }
        ToCore::CloseWindow => KIND_CLOSE_WINDOW,
        ToCore::Drawn => KIND_DRAWN,
        ToCore::InstallUpdate => KIND_INSTALL_UPDATE,
        ToCore::ChangeSetting(change) => {
            settings::push_change(&mut payload, change)?;
            KIND_CHANGE_SETTING
        }
        ToCore::Console(open) => {
            payload.push(u8::from(*open));
            KIND_CONSOLE
        }
        #[cfg(feature = "links")]
        ToCore::Point(spot) => {
            links::push_spot(&mut payload, spot.as_ref());
            KIND_POINT
        }
        #[cfg(feature = "links")]
        ToCore::Typed { number, opened } => {
            page_index(*number).ok_or(ProtocolError::BadValue("page number"))?;
            payload.extend_from_slice(&number.to_be_bytes());
            if let Some((key, version)) = opened {
                push_key(&mut payload, *key);
                payload.extend_from_slice(&version.to_be_bytes());
            }
            KIND_TYPED
        }
        #[cfg(any(feature = "links", feature = "interaction"))]
        ToCore::Focused(focused) => {
            payload.push(u8::from(*focused));
            KIND_FOCUSED
        }
        #[cfg(feature = "interaction")]
        ToCore::Editing(editing) => {
            payload.push(u8::from(*editing));
            KIND_EDITING
        }
        #[cfg(feature = "interaction")]
        ToCore::Input(intent) => {
            interaction::push_input(&mut payload, intent)?;
            KIND_INPUT
        }
        #[cfg(feature = "phone-keyboard")]
        ToCore::PhonePanel(open) => {
            payload.push(u8::from(*open));
            KIND_PHONE_PANEL
        }
        #[cfg(feature = "phone-keyboard")]
        ToCore::PhoneFinish => KIND_PHONE_FINISH,
        #[cfg(feature = "links")]
        ToCore::Confirm(id) => {
            payload.extend_from_slice(&id.to_be_bytes());
            KIND_CONFIRM
        }
        #[cfg(feature = "links")]
        ToCore::Cancel(id) => {
            payload.extend_from_slice(&id.to_be_bytes());
            KIND_CANCEL
        }
        ToCore::History(request) => {
            library::push_history_request(&mut payload, request)?;
            KIND_HISTORY
        }
        ToCore::Bookmarks(request) => {
            library::push_bookmark_request(&mut payload, request)?;
            KIND_BOOKMARKS
        }
        ToCore::Star(request) => {
            library::push_star_request(&mut payload, request)?;
            KIND_STAR
        }
        ToCore::Bar(request) => {
            library::push_bar_request(&mut payload, request);
            KIND_BAR
        }
        ToCore::Suggest(text) => {
            push_text(&mut payload, text, MAX_ADDRESS)?;
            KIND_SUGGEST
        }
        ToCore::Suggestion { number, new_tab } => {
            payload.extend_from_slice(&number.to_be_bytes());
            payload.push(u8::from(*new_tab));
            KIND_SUGGESTION
        }
        ToCore::TabList(forward) => {
            payload.push(u8::from(*forward));
            KIND_TAB_LIST
        }
        ToCore::GoTo { forward, steps } => {
            if *steps == 0 || usize::from(*steps) > library::MAX_STEPS {
                return Err(ProtocolError::BadValue("steps"));
            }
            payload.extend_from_slice(&[u8::from(*forward), *steps]);
            KIND_GO_TO
        }
        ToCore::NewPrivateTab => KIND_NEW_PRIVATE_TAB,
        ToCore::Window(window) => {
            library::push_window(&mut payload, window)?;
            KIND_WINDOW
        }
        ToCore::Cookies(request) => {
            cookies::push_request(&mut payload, request)?;
            KIND_COOKIES
        }
    };
    write_frame(out, kind, &payload)
}

/// Reads one intent from the interface, or `None` at a clean end.
///
/// # Errors
/// On input errors, a kind the interface may not send, and any payload
/// that is malformed or out of bounds.
pub fn read_from_interface(input: &mut dyn Read) -> Result<Option<ToCore>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    let mut fields = Fields::new(&payload);
    let message = match kind {
        KIND_VISIBLE => ToCore::Visible(fields.keys()?),
        KIND_CELL => {
            if payload.len() != CELL_BYTES {
                return Err(ProtocolError::BadLength { kind, length: payload.len() });
            }
            let key = fields.key()?;
            let version = fields.u32()?;
            let (column, row) = (fields.u8()?, fields.u8()?);
            if u16::from(column) >= COLUMNS || u16::from(row) >= ROWS {
                return Err(ProtocolError::BadValue("cell outside the page"));
            }
            ToCore::Cell { key, version, column, row }
        }
        KIND_SELECT_TAB => ToCore::SelectTab(fields.u8()?),
        KIND_CLOSE_TAB => ToCore::CloseTab(fields.u8()?),
        KIND_MOVE_TAB => {
            let (from, to) = (fields.u8()?, fields.u8()?);
            if usize::from(from) >= MAX_TABS || usize::from(to) >= MAX_TABS {
                return Err(ProtocolError::BadValue("tab place"));
            }
            ToCore::MoveTab { from, to }
        }
        KIND_NEW_TAB => ToCore::NewTab,
        KIND_OPENED => ToCore::Opened(if payload.is_empty() { None } else { Some(fields.key()?) }),
        KIND_MUTE_TAB => ToCore::MuteTab(fields.u8()?),
        KIND_MUTE_ALL => ToCore::MuteAll(fields.flag("mute")?),
        KIND_NAVIGATE => ToCore::Navigate(fields.text(MAX_ADDRESS)?),
        KIND_BACK => ToCore::Back,
        KIND_FORWARD => ToCore::Forward,
        KIND_BOOKMARK => ToCore::Bookmark,
        KIND_HOME => ToCore::Home,
        KIND_RELOAD => ToCore::Reload,
        KIND_PAGE_CONTROL => ToCore::PageControl(controls::read_control(&mut fields)?),
        KIND_FROM | KIND_NEW_WINDOW | KIND_TAB_TO_WINDOW | KIND_CLOSE_WINDOW => window_intent(kind, &mut fields)?,
        KIND_DRAWN | KIND_INSTALL_UPDATE => update_intent(kind),
        KIND_CHANGE_SETTING => ToCore::ChangeSetting(settings::read_change(&mut fields)?),
        KIND_CONSOLE => ToCore::Console(fields.flag("flag")?),
        #[cfg(feature = "links")]
        KIND_POINT => ToCore::Point(if payload.is_empty() { None } else { Some(links::read_spot(&mut fields)?) }),
        #[cfg(feature = "links")]
        KIND_TYPED => {
            let number = fields.display_page()?;
            let opened = if fields.is_empty() { None } else { Some((fields.key()?, fields.u32()?)) };
            ToCore::Typed { number, opened }
        }
        #[cfg(feature = "interaction")]
        KIND_INPUT => ToCore::Input(interaction::read_input(&mut fields)?),
        #[cfg(feature = "phone-keyboard")]
        KIND_PHONE_PANEL => ToCore::PhonePanel(fields.flag("phone panel")?),
        #[cfg(feature = "phone-keyboard")]
        KIND_PHONE_FINISH => ToCore::PhoneFinish,
        #[cfg(any(feature = "links", feature = "interaction"))]
        KIND_FOCUSED => ToCore::Focused(fields.flag("focus")?),
        #[cfg(feature = "interaction")]
        KIND_EDITING => ToCore::Editing(fields.flag("editing")?),
        #[cfg(feature = "links")]
        KIND_CONFIRM => ToCore::Confirm(fields.u32()?),
        #[cfg(feature = "links")]
        KIND_CANCEL => ToCore::Cancel(fields.u32()?),
        KIND_HISTORY => ToCore::History(library::read_history_request(&mut fields)?),
        KIND_BOOKMARKS => ToCore::Bookmarks(library::read_bookmark_request(&mut fields)?),
        KIND_STAR => ToCore::Star(library::read_star_request(&mut fields)?),
        KIND_BAR => ToCore::Bar(library::read_bar_request(&mut fields)?),
        KIND_SUGGEST => ToCore::Suggest(fields.text(MAX_ADDRESS)?),
        KIND_SUGGESTION => ToCore::Suggestion { number: fields.u32()?, new_tab: fields.flag("flag")? },
        KIND_TAB_LIST => ToCore::TabList(fields.flag("flag")?),
        KIND_GO_TO => {
            let forward = fields.flag("flag")?;
            ToCore::GoTo { forward, steps: library::read_steps(&mut fields)? }
        }
        KIND_NEW_PRIVATE_TAB => ToCore::NewPrivateTab,
        KIND_WINDOW => ToCore::Window(library::read_window(&mut fields)?),
        KIND_COOKIES => ToCore::Cookies(cookies::read_request(&mut fields)?),
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    fields.end()?;
    Ok(Some(message))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: PageKey = PageKey { channel: 7, page: 0x123 };

    fn strip() -> StripState {
        StripState {
            address: "https://example.invalid/".into(),
            identity: "example.invalid".into(),
            encrypted: true,
            back: true,
            forward: false,
            bookmarked: true,
            local: false,
            built_in: false,
            private: true,
            notice: "Proxy changed to socks5://127.0.0.1:1080".into(),
            refused: Some(CertificateProblem::Revoked),
        }
    }

    /// Decision 83: a refused certificate's kind is one of seven codes;
    /// any other is a broken message.
    #[test]
    fn a_certificate_problem_is_held_to_its_codes() {
        for problem in CertificateProblem::ALL {
            assert_eq!(problem_of(problem_code(problem)), Some(problem));
        }
        assert_eq!(problem_of(0), None);
        assert_eq!(problem_of(8), None);
        let mut bytes = Vec::new();
        write_to_interface(&mut bytes, &ToInterface::Strip(strip())).unwrap();
        let last = bytes.len() - 1;
        bytes[last] = 9;
        assert!(read_from_core(&mut bytes.as_slice()).is_err());
    }

    fn channel() -> Channel {
        let mut channel = Channel::empty();
        channel.channel = 9;
        channel.title = "Example".into();
        channel.banks[0] = "News".into();
        channel.pages[0] = Some(PageInfo { title: "Front page".into(), status: Status::Loaded });
        channel.pages[799] = Some(PageInfo { title: "Clock".into(), status: Status::Available });
        channel
    }

    /// Decision 72: window 0 is a new window's, never one's own; a tab's
    /// places are held to the strip's bound.
    #[test]
    fn window_messages_are_held_to_their_bounds() {
        assert!(write_to_core(&mut Vec::new(), &ToCore::From(NEW_WINDOW)).is_err());
        assert!(write_to_interface(&mut Vec::new(), &ToInterface::OpenWindow { window: NEW_WINDOW, geometry: None }).is_err());
        assert!(write_to_core(&mut Vec::new(), &ToCore::TabToWindow { tab: 64, window: 1, index: 0 }).is_err());
        let refused = |bytes: &[u8]| read_from_interface(&mut &bytes[..]).is_err();
        assert!(refused(&[KIND_FROM, 0, 0, 0, 4, 0, 0, 0, 0]), "From window 0");
        assert!(refused(&[KIND_NEW_WINDOW, 0, 0, 0, 1, 2]), "a flag of 2");
        assert!(refused(&[KIND_TAB_TO_WINDOW, 0, 0, 0, 6, 0, 0, 0, 0, 1, 65]), "a place past the strip");
        let refused = |bytes: &[u8]| read_from_core(&mut &bytes[..]).is_err();
        assert!(refused(&[KIND_TARGET, 0, 0, 0, 4, 0, 0, 0, 0]), "Target window 0");
    }

    #[test]
    fn every_message_to_the_interface_round_trips() {
        for message in [
            ToInterface::Sounding(vec![3, 7]),
            ToInterface::Tabs {
                tabs: vec![
                    TabInfo { title: "news".into(), colour: 1, channel: 3, muted: false, private: false },
                    TabInfo { title: "sport".into(), colour: 7, channel: 9, muted: true, private: true },
                ],
                active: 1,
            },
            ToInterface::Strip(strip()),
            ToInterface::Channel(channel()),
            ToInterface::Page { channel: 9, page: 0x899, title: "Clock".into() },
            ToInterface::Updated { channel: 9, page: 0x302 },
            ToInterface::Frame { key: KEY, version: 3, image: vec![1, 2, 3] },
            ToInterface::Settings(settings::SettingsView {
                home: "neoteletext:home".into(),
                sound: false,
                volume: 30,
                proxy: settings::ProxyView::None,
                local: Vec::new(),
                restore_session: true,
                cookies: settings::CookieMode::Close,
                refusal: None,
            }),
            ToInterface::Update(settings::UpdateView { version: "0.1.0".into(), status: "NeoTeletext is up to date".into(), offer: None, checking: true }),
            ToInterface::Cookies(cookies::CookiesView { query: "news".into(), total: 0, ..cookies::CookiesView::default() }),
            ToInterface::PageControls(interface_core::controls::PageControls {
                page: Some(0x302),
                fastext: [true, false, true, false, true],
                reveal: true,
                hold: false,
                mix: true,
                subpages: 4,
                chosen: Some(2),
                subcode: 3,
            }),
            ToInterface::PageControls(interface_core::controls::PageControls::default()),
            ToInterface::Remote(settings::RemotePlace { mode: settings::RemoteMode::Floating, x: 800, y: 120 }),
            ToInterface::Target(2),
            ToInterface::OpenWindow { window: 3, geometry: None },
            ToInterface::OpenWindow { window: 4, geometry: Some(library::WindowGeometry { x: -40, y: 30, width: 900, height: 700, maximised: false }) },
            ToInterface::CloseWindow(3),
            ToInterface::History(library::HistoryView { query: "news".into(), total: 0, ..library::HistoryView::default() }),
            ToInterface::Suggestions(library::Suggestions { text: "n".into(), inline: "news.example".into(), rows: Vec::new() }),
            ToInterface::TabList(library::TabList { forward: true, entries: Vec::new(), chapters: Vec::new() }),
            ToInterface::Bookmarks(library::BookmarksView::default()),
            ToInterface::Star(library::StarView { open: true, ..library::StarView::default() }),
            ToInterface::Bar(library::BarView { shown: true, ..library::BarView::default() }),
            ToInterface::Console(console::ConsoleUpdate {
                channel: 2,
                replace: false,
                entries: vec![console::ConsoleEntry { tab: console::ConsoleTab::Log, level: console::Level::Warning, at_ms: 5, text: "x".into() }],
            }),
            ToInterface::Close,
        ] {
            let mut bytes = Vec::new();
            write_to_interface(&mut bytes, &message).unwrap();
            assert_eq!(read_from_core(&mut bytes.as_slice()).unwrap().unwrap(), message);
        }
    }

    #[test]
    fn every_intent_round_trips() {
        for message in [
            ToCore::Visible(vec![KEY]),
            ToCore::Cell { key: KEY, version: 7, column: 39, row: 24 },
            ToCore::SelectTab(3),
            ToCore::CloseTab(0),
            ToCore::MoveTab { from: 3, to: 0 },
            ToCore::From(2),
            ToCore::NewWindow { private: false },
            ToCore::NewWindow { private: true },
            ToCore::TabToWindow { tab: 1, window: NEW_WINDOW, index: 0 },
            ToCore::TabToWindow { tab: 0, window: 7, index: 64 },
            ToCore::CloseWindow,
            ToCore::NewTab,
            ToCore::Navigate("sport.invalid".into()),
            ToCore::Back,
            ToCore::Forward,
            ToCore::Bookmark,
            ToCore::Home,
            ToCore::Reload,
            ToCore::PageControl(interface_core::controls::PageControl::Fastext(interface_core::controls::FastextKey::Index)),
            ToCore::PageControl(interface_core::controls::PageControl::SubPage(false)),
            ToCore::PageControl(interface_core::controls::PageControl::Latest),
            ToCore::Drawn,
            ToCore::InstallUpdate,
            ToCore::ChangeSetting(settings::SettingChange::Volume(40)),
            ToCore::Console(true),
            ToCore::History(library::HistoryRequest::Clear(library::ClearRange::All)),
            ToCore::Bookmarks(library::BookmarkRequest::Undo),
            ToCore::Star(library::StarRequest::Done),
            ToCore::Bar(library::BarRequest::Show(true)),
            ToCore::Suggest("news".into()),
            ToCore::Suggestion { number: 4, new_tab: true },
            ToCore::TabList(false),
            ToCore::GoTo { forward: true, steps: 15 },
            ToCore::NewPrivateTab,
            ToCore::Window(library::WindowGeometry { x: 10, y: 20, width: 1440, height: 900, maximised: false }),
            ToCore::Cookies(cookies::CookiesRequest::Block { number: 3, blocked: true }),
        ] {
            let mut bytes = Vec::new();
            write_to_core(&mut bytes, &message).unwrap();
            assert_eq!(read_from_interface(&mut bytes.as_slice()).unwrap().unwrap(), message);
        }
    }

    #[test]
    fn neither_side_accepts_the_other_sides_kinds() {
        // An interface sending the trusted strip's state to the trusted core.
        let mut bytes = Vec::new();
        write_to_interface(&mut bytes, &ToInterface::Strip(strip())).unwrap();
        assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(KIND_STRIP))));
        bytes.clear();
        write_to_core(&mut bytes, &ToCore::Navigate("x".into())).unwrap();
        assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(KIND_NAVIGATE))));
        // The interface sending history as if it were the trusted core's.
        bytes.clear();
        write_to_interface(&mut bytes, &ToInterface::History(library::HistoryView::default())).unwrap();
        assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(KIND_HISTORY_VIEW))));
        bytes.clear();
        write_to_core(&mut bytes, &ToCore::History(library::HistoryRequest::Panel(true))).unwrap();
        assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(KIND_HISTORY))));
        // Steps a list never offers.
        assert!(write_to_core(&mut Vec::new(), &ToCore::GoTo { forward: false, steps: 0 }).is_err());
        let beyond = u8::try_from(library::MAX_STEPS + 1).unwrap();
        assert!(read_from_interface(&mut [KIND_GO_TO, 0, 0, 0, 2, 0, beyond].as_slice()).is_err());
    }

    #[test]
    fn oversized_and_malformed_payloads_are_refused() {
        let mut bytes = vec![KIND_FRAME];
        bytes.extend_from_slice(&u32::try_from(MAX_PAYLOAD + 1).unwrap().to_be_bytes());
        assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::TooLong(_))));
        // A cell outside the 40 by 25 page.
        let mut bytes = vec![KIND_CELL, 0, 0, 0, 12, 0, 0, 0, 7, 0x01, 0x23, 0, 0, 0, 1, 40, 0];
        assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::BadValue("cell outside the page"))));
        // A page count larger than the payload holds.
        bytes = vec![KIND_VISIBLE, 0, 0, 0, 2, 0, 1];
        assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::BadValue("payload ends early"))));
        // Trailing bytes.
        bytes = vec![KIND_VISIBLE, 0, 0, 0, 3, 0, 0, 9];
        assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::BadValue("payload longer than its fields"))));
        // A page number outside 100 to 8FF.
        bytes = vec![KIND_VISIBLE, 0, 0, 0, 8, 0, 1, 0, 0, 0, 1, 0x09, 0x00];
        assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::BadValue("page number"))));
        // A new page with a hexadecimal number, and unknown strip flags.
        bytes = vec![KIND_PAGE, 0, 0, 0, 8, 0, 0, 0, 9, 0x1A, 0x00, 0, 0];
        assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::BadValue("page number"))));
        bytes = vec![KIND_STRIP, 0, 0, 0, 7, 0x80, 0, 0, 0, 0, 0, 0];
        assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::BadValue("strip flags"))));
        // A tab colour the interface has no colour for.
        bytes = vec![KIND_TABS, 0, 0, 0, 10, 0, 1, 8, 0, 0, 0, 1, 0, 0, 0];
        assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::BadValue("tab colour"))));
        // Unknown tab flags, and a mute that is neither on nor off.
        bytes = vec![KIND_TABS, 0, 0, 0, 10, 0, 1, 0, 0, 0, 0, 1, 4, 0, 0];
        assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::BadValue("tab flags"))));
        bytes = vec![KIND_MUTE_ALL, 0, 0, 0, 1, 2];
        assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::BadValue("mute"))));
    }

    /// With hyperlinks compiled out, no side accepts their kinds.
    #[cfg(not(feature = "links"))]
    #[test]
    fn with_links_compiled_out_their_kinds_are_refused() {
        for kind in 11..=15 {
            let bytes = [kind, 0, 0, 0, 0];
            assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(refused)) if refused == kind));
        }
        #[cfg(not(feature = "interaction"))]
        for kind in [26, 27] {
            let bytes = [kind, 0, 0, 0, 0];
            assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(refused)) if refused == kind));
        }
        for kind in [24, 25, 36, 37] {
            let bytes = [kind, 0, 0, 0, 0];
            assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(refused)) if refused == kind));
        }
        let bytes = [68, 0, 0, 0, 0];
        assert!(matches!(decoder::read_from_decoder(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(68))));
        let bytes = [83, 0, 0, 0, 0];
        assert!(matches!(fetcher::read_from_fetcher(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(83))));
    }

    #[test]
    fn a_page_listed_twice_is_refused() {
        let mut bytes = Vec::new();
        write_to_interface(&mut bytes, &ToInterface::Channel(channel())).unwrap();
        // Make the second entry's page number (899) equal the first's (100).
        let second = bytes.windows(2).rposition(|pair| pair == [0x08, 0x99]).unwrap();
        bytes[second] = 0x01;
        bytes[second + 1] = 0x00;
        assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::BadValue("page listed twice"))));
    }

    #[test]
    fn text_is_cleaned_before_sending_and_checked_on_arrival() {
        let hostile = "exa\u{202E}moc.\u{200B}elpmaxe\u{0007}";
        assert_eq!(clean(hostile, MAX_IDENTITY), "examoc.elpmaxe");
        assert_eq!(clean("ééé", 5), "éé", "cut on a character boundary");
        let mut bytes = Vec::new();
        let strip = ToInterface::Strip(StripState { identity: hostile.into(), ..StripState::default() });
        assert!(matches!(write_to_interface(&mut bytes, &strip), Err(ProtocolError::BadValue("text not cleaned"))));
        // Written by hand, as a faulty sender would, it is still refused.
        let identity = hostile.as_bytes();
        let mut payload = vec![0];
        payload.extend_from_slice(&u16::try_from(identity.len()).unwrap().to_be_bytes());
        payload.extend_from_slice(identity);
        payload.extend_from_slice(&[0, 0]);
        let mut bytes = vec![KIND_STRIP];
        bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
        bytes.extend_from_slice(&payload);
        assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::BadValue("text not cleaned"))));
    }
}
