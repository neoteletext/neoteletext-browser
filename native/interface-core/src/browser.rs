//! The browser window, after the owner's mockups: the tab strip and
//! window buttons along the top as Chrome draws them, the toolbar with the
//! trusted strip's address field, and below it the page banks on the left,
//! the chosen bank's page list in the middle, the zoomable page area on the
//! right, and the overview of all 800 pages along the bottom.
//!
//! The browser holds only what it is shown. What it shows comes from the
//! trusted core; what the user does leaves as [`Output`]s, which the host
//! turns into messages (design section 5.2).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

use crate::browser::regions::Regions;
use crate::canvas::PageKey;
use crate::channel::Channel;
use crate::editor::Editor;
use crate::geometry::{Point, Rect};
use crate::wall::Wall;

mod address;
mod banks;
mod bookmarks;
mod bookmarks_bar;
mod certificate_refused;
mod console_panel;
mod cookies_tab;
mod divider;
#[cfg(test)]
mod fixture;
mod focus;
mod frames;
mod full_screen;
mod glyphs;
mod history;
#[cfg(feature = "links")]
mod hyperlinks;
mod keys;
mod list_focus;
mod menu;
mod output;
mod overview;
mod page_controls;
mod page_list;
mod pages;
#[cfg(feature = "phone-keyboard")]
mod phone;
#[cfg(feature = "phone-keyboard")]
mod phone_keys;
mod pointer;
mod popups;
mod regions;
mod remote;
mod settings_panel;
mod shown;
mod star;
mod subpage_list;
mod suggestions;
mod tab_lists;
mod tab_strip;
mod tips;
mod toolbar;
#[cfg(feature = "interaction")]
mod typing;
mod widgets;
mod window;

pub use keys::{Key, Modifiers};
pub use output::{Output, WindowAction};
#[cfg(feature = "phone-keyboard")]
pub use phone::{PhoneStatus, PhoneUiView};
#[cfg(feature = "phone-keyboard")]
pub use phone_keys::{PhonePageKey, phone_page_key};
pub use pointer::Cursor;
pub use shown::{CertificateProblem, Frame, StripState, TabInfo};
#[cfg(feature = "interaction")]
pub use typing::{CursorView, TypingView};

/// The address field's limit, as the trusted core's address rules.
pub const MAX_ADDRESS: usize = 2048;

/// Scrolling by one wheel line.
pub const LINE_HEIGHT: f64 = 21.0;

/// The overview of every page as the viewer has it.
#[derive(Clone, Copy, Debug, Default)]
struct OverviewState {
    /// Folded to its heading.
    folded: bool,
    /// Taken off the window from the View menu (decision 109).
    hidden: bool,
}

/// The whole browser window: tabs, trusted strip, banks, page list, page
/// area and overview. It draws through a [`crate::canvas::Canvas`] and reports what the
/// user did as [`Output`]s; it holds no state the trusted core owns.
pub struct Browser {
    frame: Frame,
    tabs: Vec<TabInfo>,
    active: usize,
    /// The channels playing sound, muted or not.
    sounding: BTreeSet<u32>,
    strip: StripState,
    channel: Channel,
    bank: usize,
    selected: Option<usize>,
    list_scroll: f64,
    wall: Wall,
    editor: Editor,
    editing: bool,
    field_scroll: f64,
    digits: String,
    pulses: Vec<(usize, u64)>,
    /// When each recently updated page had its new version.
    updates: Vec<(usize, u64)>,
    /// The overview folded into the footer: it then draws nothing but its
    /// heading and keeps no animation.
    overview: OverviewState,
    /// The page row the pointer is over, by page index, if any.
    hovered_row: Option<usize>,
    /// The magazine the pointer is over, if any.
    hovered_bank: Option<usize>,
    /// The sub-pages under the page list (decision 111).
    subpages: subpage_list::SubPages,
    /// The list at the left that has the keys, if one has.
    list_focus: Option<list_focus::ListFocus>,
    /// The page list's width, as the divider was pulled.
    list_width: f64,
    /// Where the divider was gripped, from its left edge, while it is pulled.
    divider_grip: Option<f64>,
    /// The overview's page under the pointer, shown at its native size.
    previewed: Option<usize>,
    pointer: Point,
    visible: Vec<PageKey>,
    /// The version of each page's latest image, as the trusted core sent
    /// it: a click names the image it was made on.
    versions: BTreeMap<PageKey, u32>,
    regions: Regions,
    outputs: Vec<Output>,
    /// The window below the strip, where the settings panel is centred.
    body: Rect,
    /// The settings panel (requirement 19.3).
    settings: settings_panel::SettingsUi,
    /// The developer console (requirement 19.8).
    console: console_panel::ConsoleUi,
    /// The history panel (requirement 20.4).
    history: history::HistoryUi,
    /// The cookies panel (requirement 21.6).
    cookies: cookies_tab::CookiesUi,
    /// What each control is, shown when the pointer rests on it.
    tips: tips::TipsUi,
    /// The open page's controls (decision 67).
    page_controls: page_controls::ControlsUi,
    /// The remote control (decision 68).
    remote: remote::RemoteUi,
    /// The certificate-error screen's buttons (decision 83).
    refused: certificate_refused::RefusedUi,
    /// The tab strip's overflow and dragging (decision 69).
    tab_strip: tab_strip::TabsUi,
    /// Full screen, the page only (decision 85).
    full_screen: full_screen::FullScreenUi,
    /// The bookmarks manager (requirement 20.17).
    bookmarks: bookmarks::BookmarksUi,
    /// The star's popover (requirement 20.16).
    star: star::StarUi,
    /// The bookmarks bar (requirement 20.18).
    bar: bookmarks_bar::BarUi,
    /// The menu button's menu.
    menu: menu::MenuUi,
    /// Back's and Forward's lists.
    tab_lists: tab_lists::TabListUi,
    /// The address field's suggestions.
    suggestions: suggestions::SuggestionsUi,
    /// The last press asked for a new tab: Shift was held.
    press_new_tab: bool,
    /// Hyperlinks (`hyperlinks.md`).
    #[cfg(feature = "links")]
    links: hyperlinks::LinkUi,
    /// Typing into a page (`interaction.md`).
    #[cfg(feature = "interaction")]
    typing: typing::TerminalUi,
    /// The phone keyboard (INPUT.md section 10).
    #[cfg(feature = "phone-keyboard")]
    phone: phone::PhoneUi,
}

impl Browser {
    /// A browser with no tabs and an empty channel, drawn in `frame`.
    #[must_use]
    pub fn new(frame: Frame) -> Self {
        Self {
            frame,
            tabs: Vec::new(),
            active: 0,
            sounding: BTreeSet::new(),
            strip: StripState::default(),
            channel: Channel::empty(),
            bank: 0,
            selected: None,
            list_scroll: 0.0,
            wall: Wall::new(),
            editor: Editor::new(MAX_ADDRESS),
            editing: false,
            field_scroll: 0.0,
            digits: String::new(),
            pulses: Vec::new(),
            updates: Vec::new(),
            overview: OverviewState::default(),
            hovered_row: None,
            hovered_bank: None,
            subpages: subpage_list::SubPages::default(),
            list_focus: None,
            list_width: divider::LIST_WIDTH,
            divider_grip: None,
            previewed: None,
            pointer: Point::ORIGIN,
            visible: Vec::new(),
            versions: BTreeMap::new(),
            regions: Regions::default(),
            outputs: Vec::new(),
            body: Rect::ZERO,
            settings: settings_panel::SettingsUi::default(),
            console: console_panel::ConsoleUi::default(),
            history: history::HistoryUi::default(),
            cookies: cookies_tab::CookiesUi::default(),
            tips: tips::TipsUi::default(),
            page_controls: page_controls::ControlsUi::default(),
            remote: remote::RemoteUi::default(),
            refused: certificate_refused::RefusedUi::default(),
            tab_strip: tab_strip::TabsUi::default(),
            full_screen: full_screen::FullScreenUi::default(),
            bookmarks: bookmarks::BookmarksUi::default(),
            star: star::StarUi::default(),
            bar: bookmarks_bar::BarUi::default(),
            menu: menu::MenuUi::default(),
            tab_lists: tab_lists::TabListUi::default(),
            suggestions: suggestions::SuggestionsUi::default(),
            press_new_tab: false,
            #[cfg(feature = "links")]
            links: hyperlinks::LinkUi::default(),
            #[cfg(feature = "interaction")]
            typing: typing::TerminalUi::default(),
            #[cfg(feature = "phone-keyboard")]
            phone: phone::PhoneUi::default(),
        }
    }

    /// A new image of a page arrived, of this version.
    pub fn frame_version(&mut self, key: PageKey, version: u32) {
        self.versions.insert(key, version);
        // The links under the pointer are now those of the new image.
        #[cfg(feature = "links")]
        self.repoint();
    }

    /// What the trusted strip shows, as the trusted core decides it.
    pub fn set_strip(&mut self, strip: StripState) {
        self.strip = strip;
    }

    /// The page area.
    #[must_use]
    pub fn wall(&self) -> &Wall {
        &self.wall
    }

    /// The page area, to change.
    pub fn wall_mut(&mut self) -> &mut Wall {
        &mut self.wall
    }

    /// Everything the user did since the last call.
    pub fn take_outputs(&mut self) -> Vec<Output> {
        core::mem::take(&mut self.outputs)
    }

    // ---- Input ----

    // ---- Drawing ----
}
