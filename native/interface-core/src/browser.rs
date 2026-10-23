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
mod bookmarks_ui;
mod console_ui;
mod controls_ui;
mod cookies_ui;
#[cfg(test)]
mod fixture;
mod focus;
mod frames;
mod glyphs;
mod history_ui;
mod keys;
#[cfg(feature = "links")]
mod link_ui;
mod output;
mod overview;
mod page_list;
mod pages;
#[cfg(feature = "phone-keyboard")]
mod phone_ui;
mod pointer;
mod popup_ui;
mod refused_ui;
mod regions;
mod remote_ui;
mod settings_ui;
mod shown;
mod star_ui;
mod tab_strip;
#[cfg(feature = "interaction")]
mod terminal_ui;
mod tips_ui;
mod toolbar;
mod widgets;
mod window;

pub use keys::{Key, Modifiers};
pub use output::{Output, WindowAction};
#[cfg(feature = "phone-keyboard")]
pub use phone_ui::{PhoneStatus, PhoneUiView};
pub use pointer::Cursor;
pub use shown::{CertificateProblem, Frame, StripState, TabInfo};
#[cfg(feature = "interaction")]
pub use terminal_ui::{CursorView, TypingView};

/// The address field's limit, as the trusted core's address rules.
pub const MAX_ADDRESS: usize = 2048;

/// Scrolling by one wheel line.
pub const LINE_HEIGHT: f64 = 21.0;

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
    overview_folded: bool,
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
    settings: settings_ui::SettingsUi,
    /// The developer console (requirement 19.8).
    console: console_ui::ConsoleUi,
    /// The history panel (requirement 20.4).
    history: history_ui::HistoryUi,
    /// The cookies panel (requirement 21.6).
    cookies: cookies_ui::CookiesUi,
    /// What each control is, shown when the pointer rests on it.
    tips: tips_ui::TipsUi,
    /// The open page's controls (decision 67).
    page_controls: controls_ui::ControlsUi,
    /// The remote control (decision 68).
    remote: remote_ui::RemoteUi,
    /// The certificate-error screen's buttons (decision 83).
    refused: refused_ui::RefusedUi,
    /// The tab strip's overflow and dragging (decision 69).
    tab_strip: tab_strip::TabsUi,
    /// The bookmarks manager (requirement 20.17).
    bookmarks: bookmarks_ui::BookmarksUi,
    /// The star's popover (requirement 20.16).
    star: star_ui::StarUi,
    /// The bookmarks bar (requirement 20.18).
    bar: star_ui::BarUi,
    /// The menu, Back's and Forward's lists, and the suggestions.
    popups: popup_ui::PopupUi,
    /// The last press asked for a new tab: Shift was held.
    press_new_tab: bool,
    /// Hyperlinks (`hyperlinks.md`).
    #[cfg(feature = "links")]
    links: link_ui::LinkUi,
    /// Typing into a page (`interaction.md`).
    #[cfg(feature = "interaction")]
    terminal: terminal_ui::TerminalUi,
    /// The phone keyboard (INPUT.md section 10).
    #[cfg(feature = "phone-keyboard")]
    phone: phone_ui::PhoneUi,
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
            overview_folded: false,
            previewed: None,
            pointer: Point::ORIGIN,
            visible: Vec::new(),
            versions: BTreeMap::new(),
            regions: Regions::default(),
            outputs: Vec::new(),
            body: Rect::ZERO,
            settings: settings_ui::SettingsUi::default(),
            console: console_ui::ConsoleUi::default(),
            history: history_ui::HistoryUi::default(),
            cookies: cookies_ui::CookiesUi::default(),
            tips: tips_ui::TipsUi::default(),
            page_controls: controls_ui::ControlsUi::default(),
            remote: remote_ui::RemoteUi::default(),
            refused: refused_ui::RefusedUi::default(),
            tab_strip: tab_strip::TabsUi::default(),
            bookmarks: bookmarks_ui::BookmarksUi::default(),
            star: star_ui::StarUi::default(),
            bar: star_ui::BarUi::default(),
            popups: popup_ui::PopupUi::default(),
            press_new_tab: false,
            #[cfg(feature = "links")]
            links: link_ui::LinkUi::default(),
            #[cfg(feature = "interaction")]
            terminal: terminal_ui::TerminalUi::default(),
            #[cfg(feature = "phone-keyboard")]
            phone: phone_ui::PhoneUi::default(),
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
