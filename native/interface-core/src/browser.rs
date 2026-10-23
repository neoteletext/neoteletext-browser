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

use crate::browser::glyphs::{WINDOW_GLYPH, WINDOW_GLYPH_THICKNESS};
use crate::browser::regions::Regions;
use crate::canvas::{Canvas, PageKey};
use crate::channel::{Channel, PAGES_PER_BANK, page_index, page_number};
use crate::editor::Editor;
use crate::geometry::{Point, Rect};
use crate::theme::{BLACK, GREEN, RED, YELLOW};
use crate::ui::border;
use crate::wall::{CellClick, Wall};

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
#[cfg(feature = "links")]
mod link_ui;
mod output;
mod overview;
mod page_list;
mod pages;
#[cfg(feature = "phone-keyboard")]
mod phone_ui;
mod popup_ui;
mod refused_ui;
mod regions;
mod remote_ui;
mod settings_ui;
mod shown;
mod star_ui;
mod tabs_ui;
#[cfg(feature = "interaction")]
mod terminal_ui;
mod tips_ui;
mod toolbar;
mod widgets;
mod window;

pub use output::{Output, WindowAction};
#[cfg(feature = "phone-keyboard")]
pub use phone_ui::{PhoneStatus, PhoneUiView};
pub use shown::{CertificateProblem, Frame, StripState, TabInfo};
#[cfg(feature = "interaction")]
pub use terminal_ui::{CursorView, TypingView};

/// The address field's limit, as the trusted core's address rules.
pub const MAX_ADDRESS: usize = 2048;

// ---- Tab strip ----
const STRIP_HEIGHT: f64 = 44.0;
const TAB_TOP: f64 = 6.0;
const TAB_TOP_INACTIVE: f64 = 9.0;
const TAB_MAX_WIDTH: f64 = 230.0;
const TAB_MIN_WIDTH: f64 = 90.0;
const TAB_GAP: f64 = 4.0;
const TAB_TEXT_INSET: f64 = 14.0;
const TAB_CLOSE: f64 = 22.0;
/// A private tab's mark, before its title.
const PRIVATE_MARK: f64 = 10.0;
/// A tab's sound button, before its close button, as Chrome places it.
const TAB_SPEAKER: f64 = 24.0;
const NEW_TAB: f64 = 38.0;
const STRIP_MARGIN: f64 = 8.0;
const WINDOW_BUTTON_WIDTH: f64 = 44.0;
const WINDOW_BUTTON_HEIGHT: f64 = 32.0;
const WINDOW_BUTTON_GAP: f64 = 6.0;
// ---- Toolbar ----
// ---- Panels ----
// ---- Overview ----
/// Scrolling by one wheel line.
pub const LINE_HEIGHT: f64 = 21.0;
/// A key the browser handles, as the host translates it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key<'a> {
    /// Text typed or composed by an input method.
    Text(&'a str),
    /// Return or Enter.
    Enter,
    /// Escape.
    Escape,
    /// Backspace: deletes before the caret.
    Backspace,
    /// Delete: deletes after the caret.
    Delete,
    /// The left arrow.
    Left,
    /// The right arrow.
    Right,
    /// The up arrow.
    Up,
    /// The down arrow.
    Down,
    /// Home.
    Home,
    /// End.
    End,
    /// Tab: with Shift, backwards.
    Tab,
}

/// The shape the pointer should take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    /// The ordinary pointer.
    Arrow,
    /// Over something a click acts on.
    Hand,
    /// Over a link that leaves the channel: the shape of a shortcut.
    External,
}

/// The modifier keys. `command` is Command on macOS and Control elsewhere.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools, reason = "independent keys, each held or not, as the platform reports them")]
pub struct Modifiers {
    /// Shift is held.
    pub shift: bool,
    /// Command (macOS) or Control (elsewhere) is held.
    pub command: bool,
    /// Option (macOS) or Alt (elsewhere) is held.
    pub alt: bool,
    /// The key is held down and repeating.
    pub repeat: bool,
}

/// The whole browser window: tabs, trusted strip, banks, page list, page
/// area and overview. It draws through a [`Canvas`] and reports what the
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
    tab_strip: tabs_ui::TabsUi,
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
            tab_strip: tabs_ui::TabsUi::default(),
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

    /// The tabs as the trusted core reports them, and which is active.
    pub fn set_tabs(&mut self, tabs: Vec<TabInfo>, active: usize) {
        self.active = active.min(tabs.len().saturating_sub(1));
        self.tabs = tabs;
    }

    /// The channels playing sound, muted or not, as the host's mixers
    /// report. Returns whether that changed what the tabs show.
    pub fn set_sounding(&mut self, sounding: BTreeSet<u32>) -> bool {
        let changed = sounding != self.sounding;
        self.sounding = sounding;
        changed
    }

    /// Whether the tab plays sound, muted or not.
    #[must_use]
    pub fn tab_sounding(&self, tab: &TabInfo) -> bool {
        self.sounding.contains(&tab.channel)
    }

    /// Whether every tab is muted: the mute-all button then unmutes them.
    #[must_use]
    pub fn all_muted(&self) -> bool {
        !self.tabs.is_empty() && self.tabs.iter().all(|tab| tab.muted)
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

    /// The pointer moved to `x`, `y` in logical pixels; returns whether
    /// the window needs drawing again.
    pub fn pointer_moved(&mut self, x: f64, y: f64, now: u64) -> bool {
        self.pointer = Point::new(x, y);
        if self.remote_moved(self.pointer) {
            return true;
        }
        let dragged = self.wall.pointer_moved(self.pointer, now);
        let hovered = self.wall.hover(self.pointer, now);
        let previewed = self.overview_page(self.pointer);
        let preview_changed = previewed != self.previewed;
        #[cfg(feature = "interaction")]
        self.terminal_moved();
        self.previewed = previewed;
        #[cfg(feature = "links")]
        let linked = self.repoint();
        #[cfg(not(feature = "links"))]
        let linked = false;
        let tipped = self.tips_moved(now);
        let tabbed = self.tabs_moved(self.pointer);
        dragged || hovered || preview_changed || linked || tipped || tabbed
    }

    /// The pointer's shape where it is: a hand over a page a click would
    /// zoom or choose and over the overview's fold button, the arrow
    /// elsewhere.
    #[must_use]
    pub fn cursor(&self) -> Cursor {
        #[cfg(feature = "links")]
        if let Some(cursor) = self.link_cursor() {
            return cursor;
        }
        let over_toggle = self.regions.overview_toggle.contains(self.pointer);
        if self.wall.over_page() || self.previewed.is_some() || over_toggle { Cursor::Hand } else { Cursor::Arrow }
    }

    /// The main button was released.
    pub fn pointer_released(&mut self) {
        self.remote_released();
        self.tabs_released();
        self.release_back_or_forward();
        let new_tab = self.press_new_tab;
        self.bookmarks_released(new_tab);
        self.wall.pointer_released();
        // A button the channel saw pressed is released to it.
        #[cfg(feature = "interaction")]
        self.terminal_button(neoteletext_input::event::MouseAction::Release, 0);
    }

    /// A press of the main button, with `extend` when Shift is held.
    #[allow(clippy::too_many_lines, reason = "one hit-test per part of the window, in drawing order")]
    pub fn pointer_pressed<C: Canvas + ?Sized>(&mut self, canvas: &mut C, extend: bool, now: u64) -> bool {
        self.tips_pressed();
        // A press means the button was let go since the last: a drag whose
        // release never came, as one let go outside every window, is over.
        self.tabs_cancel();
        // Shift opens what a press opens in a new tab.
        self.press_new_tab = extend;
        // The drop-downs and the star's popover are over everything; the
        // panels over everything below the strip.
        if self.popups_press(self.pointer, extend) || self.tabs_press(self.pointer) || self.star_press(self.pointer) {
            return true;
        }
        if self.history_press(self.pointer, extend) || self.bookmarks_press(self.pointer) {
            return true;
        }
        if self.settings_press(self.pointer) {
            return true;
        }
        if self.console_press(self.pointer) {
            return true;
        }
        #[cfg(feature = "phone-keyboard")]
        if self.phone_press(self.pointer) {
            return true;
        }
        // The strip's question, drawn over the address field.
        #[cfg(feature = "links")]
        if self.link_press(now) {
            return true;
        }
        if self.refused_press(self.pointer) {
            return true;
        }
        if self.remote_press(self.pointer, now) {
            return true;
        }
        let point = self.pointer;
        let regions = &self.regions;
        if regions.field.contains(point) {
            if self.editing {
                let at = self.offset_at(canvas, point.x);
                self.editor.place(at, extend);
            } else {
                self.start_editing();
            }
            return true;
        }
        self.stop_editing();
        let regions = &self.regions;
        // Back and Forward act when released, or open their lists when held.
        for (rect, forward, enabled) in [(regions.back, false, self.strip.back), (regions.forward, true, self.strip.forward)] {
            if rect.contains(point) {
                if enabled {
                    self.hold_back_or_forward(forward, now);
                }
                return true;
            }
        }
        if regions.gear.contains(point) {
            self.toggle_settings();
            return true;
        }
        if regions.menu.contains(point) {
            self.toggle_menu();
            return true;
        }
        let mute_all = Output::MuteAll(!self.all_muted());
        let action = [
            (regions.new_tab, Output::NewTab),
            (regions.reload, Output::Reload),
            (regions.home, Output::Home),
            (regions.star, Output::Bookmark),
            (regions.mute_all, mute_all),
            (regions.minimise, Output::Window(WindowAction::Minimise)),
            (regions.maximise, Output::Window(WindowAction::Maximise)),
            (regions.close, Output::Window(WindowAction::Close)),
        ]
        .into_iter()
        .find(|(rect, _)| rect.contains(point));
        if let Some((_, output)) = action {
            self.outputs.push(output);
            return true;
        }
        if self.bar_press(point, extend) {
            return true;
        }
        let regions = &self.regions;
        if regions.strip.contains(point) {
            self.outputs.push(Output::Window(WindowAction::Drag));
            return false;
        }
        if let Some(bank) = regions.banks.iter().position(|rect| rect.contains(point)) {
            self.choose_bank(bank);
            return true;
        }
        if regions.list.contains(point) {
            if let Some(&(_, index)) = regions.rows.iter().find(|(rect, _)| rect.contains(point)) {
                self.select_page(index, true);
            }
            return true;
        }
        if regions.overview_toggle.contains(point) {
            self.toggle_overview();
            return true;
        }
        if regions.cells.contains(point) {
            if let Some(index) = self.cell_index(point)
                && matches!(self.channel.pages.get(index), Some(Some(_)))
            {
                self.select_page(index, true);
            }
            return true;
        }
        if self.controls_press(point) {
            return true;
        }
        if self.wall.pointer_pressed(point, now) {
            return true;
        }
        if let Some(click) = self.wall.cell_at(point) {
            if let Some(index) = page_index(click.key.page) {
                self.selected = Some(index);
                self.reveal_row(index);
            }
            // A page on the wall zooms to fill the area; on a zoomed page a
            // click is on one of its cells, where links will be followed.
            // On a page whose channel or page asked for the mouse a plain
            // click is the host's, as a terminal with mouse reporting gives
            // it; the browser's modifier keeps it the browser's (INPUT.md
            // section 6.5, REQ-INT-115).
            #[cfg(feature = "interaction")]
            if self.wall.zoomed().is_some() && self.click_is_hosts() {
                return self.terminal_button(neoteletext_input::event::MouseAction::Press, 0);
            }
            if self.wall.zoomed().is_some() {
                let version = self.versions.get(&click.key).copied().unwrap_or_default();
                self.outputs.push(Output::Cell(CellClick { version, ..click }));
            } else {
                self.wall.zoom(click.key, now);
            }
            return true;
        }
        false
    }

    /// A mouse wheel turned by `lines`, as the platform reports a wheel
    /// that moves in notches (towards the user positive). Over a zoomed
    /// page each notch is one page (`Wall::wheel_notch`); elsewhere it
    /// scrolls by lines.
    pub fn wheel_lines(&mut self, lines: f64, now: u64) -> bool {
        if self.wall.zoomed().is_some() && self.wall.contains(self.pointer) && !self.wall.over_bar(self.pointer) {
            if self.wall.wheel_notch(lines, now).is_some() {
                self.follow_zoom();
            }
            return true;
        }
        self.scroll_by(lines * LINE_HEIGHT, now)
    }

    /// A press of the middle button: on a tab, the tab closes, and on what
    /// opens a channel, it opens in a new tab, as in every current browser.
    pub fn middle_pressed(&mut self) -> bool {
        self.tips_pressed();
        if let Some(region) = self.regions.tabs.iter().rev().find(|region| region.tab.contains(self.pointer)) {
            self.outputs.push(Output::CloseTab(region.index));
            return true;
        }
        self.open_in_new_tab()
    }

    /// A press of the main button with Command (Control elsewhere): what
    /// opens a channel opens it in a new tab, as Shift does. Anywhere else
    /// it is not taken, and is an ordinary press.
    pub fn new_tab_pressed(&mut self) -> bool {
        self.tips_pressed();
        self.open_in_new_tab()
    }

    /// Opens what is under the pointer in a new tab: a suggestion, a
    /// bookmark of the bar or its list, a history row or a bookmark of the
    /// manager, in the order they lie over each other.
    fn open_in_new_tab(&mut self) -> bool {
        let point = self.pointer;
        let output = if let Some(number) = self.suggestion_at(point) {
            self.stop_editing();
            Output::Suggestion { number, new_tab: true }
        } else if let Some(number) = self.bar_bookmark_at(point) {
            Output::Bar(crate::library::BarRequest::Open { number, new_tab: true })
        } else if let Some(number) = self.history_row_at(point) {
            Output::History(crate::library::HistoryRequest::Open { number, new_tab: true })
        } else if let Some(number) = self.bookmark_at(point) {
            Output::Bookmarks(crate::library::BookmarkRequest::Open { number, new_tab: true })
        } else {
            return false;
        };
        self.outputs.push(output);
        true
    }

    /// A secondary click: on Back or Forward, its list is asked for
    /// (requirement 20.10).
    pub fn secondary_pressed(&mut self) -> bool {
        for (rect, forward, enabled) in [(self.regions.back, false, self.strip.back), (self.regions.forward, true, self.strip.forward)] {
            if rect.contains(self.pointer) {
                if enabled {
                    self.ask_tab_list(forward);
                }
                return true;
            }
        }
        // Anywhere else, an open list closes.
        self.close_popup()
    }

    /// Scrolls whatever is under the pointer.
    pub fn scroll_by(&mut self, distance: f64, now: u64) -> bool {
        if self.history_scroll(distance) || self.cookies_scroll(distance) || self.bookmarks_scroll(distance) {
            return true;
        }
        if self.console_scroll(distance) {
            return true;
        }
        if self.wall.over_bar(self.pointer) {
            self.wall.wheel_bar(distance, now);
            return true;
        }
        // Zoomed, the wheel moves from page to page.
        if self.wall.zoomed().is_some() && self.wall.contains(self.pointer) {
            if self.wall.wheel_pages(distance, now).is_some() {
                self.follow_zoom();
            }
            return true;
        }
        if self.regions.list.contains(self.pointer) {
            self.list_scroll = (self.list_scroll + distance).max(0.0);
            return true;
        }
        if self.wall.contains(self.pointer) {
            self.wall.scroll_by(distance);
            return true;
        }
        false
    }

    /// Returns whether the window needs drawing again.
    pub fn key(&mut self, key: Key<'_>, modifiers: Modifiers, now: u64) -> bool {
        if modifiers.command {
            return self.shortcut(key, modifiers, now);
        }
        // Alt-Home, the Home button's key away from macOS (requirement 19.6).
        if modifiers.alt && key == Key::Home {
            self.outputs.push(Output::Home);
            return true;
        }
        if key == Key::Escape && (self.close_popup() || self.tabs_cancel()) {
            return true;
        }
        if let Some(changed) = self.star_key(key, modifiers.shift) {
            return changed;
        }
        if let Some(changed) = self.history_key(key, modifiers.shift) {
            return changed;
        }
        if let Some(changed) = self.cookies_key(key, modifiers.shift) {
            return changed;
        }
        if let Some(changed) = self.console_key(key, modifiers.shift) {
            return changed;
        }
        if let Some(changed) = self.bookmarks_key(key, modifiers.shift) {
            return changed;
        }
        if let Some(changed) = self.settings_key(key, modifiers.shift) {
            return changed;
        }
        if self.editing {
            return self.edit(key, modifiers.shift);
        }
        #[cfg(feature = "links")]
        if let Some(changed) = self.link_key(key, modifiers, now) {
            return changed;
        }
        if let Some(changed) = self.control_key(key) {
            return changed;
        }
        match key {
            Key::Escape => return self.wall.unzoom(now),
            Key::Left => self.wall.step(-1, now),
            Key::Right => self.wall.step(1, now),
            Key::Up | Key::Down => self.step_page(key == Key::Down, now),
            Key::Text(text) => {
                for digit in text.chars().filter(char::is_ascii_digit) {
                    self.type_digit(digit);
                }
            }
            _ => return false,
        }
        true
    }

    fn shortcut(&mut self, key: Key<'_>, modifiers: Modifiers, now: u64) -> bool {
        // The browser's Escape: leaves a page whose channel has every
        // other key (INPUT.md section 9.1).
        if key == Key::Escape {
            self.stop_editing();
            return self.wall.unzoom(now);
        }
        // Clear history: Command-Shift-Delete (section 6).
        if matches!(key, Key::Delete | Key::Backspace) && modifiers.shift {
            self.clear_history_choices();
            return true;
        }
        // The tab before or after, shown or not (decision 69): Command-Option
        // and an arrow, Control-Alt elsewhere.
        if modifiers.alt && matches!(key, Key::Left | Key::Right) {
            let count = self.tabs.len();
            if count > 0 {
                let next = if key == Key::Right { self.active.saturating_add(1) % count } else { self.active.checked_sub(1).unwrap_or(count - 1) };
                self.outputs.push(Output::SelectTab(next));
            }
            return true;
        }
        let Key::Text(text) = key else { return false };
        // Command-1 to 8 choose those tabs, Command-9 the last, as in Chrome.
        if let Some(digit) = text.chars().next().and_then(|first| first.to_digit(10)).filter(|digit| (1..=9).contains(digit) && text.len() == 1) {
            let index = if digit == 9 { self.tabs.len().checked_sub(1) } else { usize::try_from(digit - 1).ok().filter(|index| *index < self.tabs.len()) };
            if let Some(index) = index {
                self.outputs.push(Output::SelectTab(index));
            }
            return true;
        }
        match text.to_ascii_lowercase().as_str() {
            "l" => self.start_editing(),
            "a" => {
                let Some(field) = self.focused_field() else { return false };
                field.select_all();
            }
            // The settings panel (requirement 19.3), Home (requirement
            // 19.6) and the console (requirement 19.8): Command-comma,
            // Command-Shift-H and Command-Option-I on macOS; Control-comma
            // and Control-Shift-I elsewhere, where Home is Alt-Home.
            "," => self.toggle_settings(),
            "h" if modifiers.shift => self.outputs.push(Output::Home),
            // Reload: Command-R on macOS, Control-R elsewhere.
            // The remote control (decision 68): Command-Shift-R.
            "r" if modifiers.shift => self.toggle_remote(now),
            "r" => self.outputs.push(Output::Reload),
            "i" if modifiers.alt || modifiers.shift => self.toggle_console(),
            // History, bookmarks and private tabs (`history-and-bookmarks.md`
            // section 6): each platform's keys are taken everywhere.
            "y" | "h" => self.toggle_history(),
            "o" if modifiers.shift => self.toggle_bookmarks(),
            "b" if modifiers.alt => self.toggle_bookmarks(),
            "b" if modifiers.shift => self.toggle_bar(),
            "t" if modifiers.shift => self.outputs.push(Output::History(crate::library::HistoryRequest::Reopen(None))),
            "n" if modifiers.shift => self.outputs.push(Output::NewPrivateTab),
            "t" => self.outputs.push(Output::NewTab),
            "n" => self.outputs.push(Output::NewWindow { private: false }),
            "w" if modifiers.shift => self.outputs.push(Output::Window(WindowAction::Close)),
            "w" => self.outputs.push(Output::CloseTab(self.active)),
            "[" if self.strip.back => self.outputs.push(Output::Back),
            "]" if self.strip.forward => self.outputs.push(Output::Forward),
            "d" => self.outputs.push(Output::Bookmark),
            _ => return false,
        }
        true
    }

    // ---- Drawing ----

    fn draw_tab_strip<C: Canvas + ?Sized>(&mut self, canvas: &mut C, width: f64) {
        canvas.fill(Rect::new(0.0, 0.0, width, STRIP_HEIGHT), 0.0, BLACK);
        let mut right = width - STRIP_MARGIN;
        if self.frame.window_buttons {
            let top = (STRIP_HEIGHT - WINDOW_BUTTON_HEIGHT) / 2.0;
            let button = |right: &mut f64| {
                let rect = Rect::new(*right - WINDOW_BUTTON_WIDTH, top, *right, top + WINDOW_BUTTON_HEIGHT);
                *right = rect.x0 - WINDOW_BUTTON_GAP;
                rect
            };
            self.regions.close = button(&mut right);
            self.regions.maximise = button(&mut right);
            self.regions.minimise = button(&mut right);
            draw_window_buttons(canvas, &self.regions);
        }
        let left = self.frame.left_inset + STRIP_MARGIN;
        self.draw_tabs(canvas, left, right);
        self.regions.strip = Rect::new(0.0, 0.0, width, STRIP_HEIGHT);
    }
}

fn draw_window_buttons<C: Canvas + ?Sized>(canvas: &mut C, regions: &Regions) {
    for (rect, colour) in [(regions.minimise, GREEN), (regions.maximise, YELLOW), (regions.close, RED)] {
        canvas.fill(rect, 0.0, colour);
    }
    let centre = |rect: Rect| Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1));
    let half = WINDOW_GLYPH / 2.0;
    let thick = WINDOW_GLYPH_THICKNESS;
    let minimise = centre(regions.minimise);
    canvas.fill(Rect::new(minimise.x - half, minimise.y + half - thick, minimise.x + half, minimise.y + half), 0.0, BLACK);
    let maximise = centre(regions.maximise);
    border(canvas, Rect::new(maximise.x - half, maximise.y - half, maximise.x + half, maximise.y + half), thick, BLACK);
    // The close cross: two bars, each a thin parallelogram from corner to corner.
    let close = centre(regions.close);
    let normal = thick / 2.0 / core::f64::consts::SQRT_2;
    for direction in [1.0, -1.0] {
        let along = Point::new(half, half * direction);
        let across = Point::new(-normal * direction, normal);
        canvas.polygon(&[close - along + across, close + along + across, close + along - across, close - along - across], BLACK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::fixture::{HEIGHT, WIDTH, browser, press_at};
    use crate::browser::regions::TabRegion;

    #[test]
    fn a_tab_playing_sound_shows_a_speaker_that_mutes_it() {
        let (mut browser, mut canvas) = browser();
        browser.take_outputs();
        assert!(browser.regions.tabs.iter().all(|region| region.speaker.is_none()), "no sound, no speaker");
        assert!(browser.set_sounding(BTreeSet::from([8])));
        assert!(!browser.set_sounding(BTreeSet::from([8])), "the same again changes nothing");
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        assert!(browser.regions.tabs[0].speaker.is_none());
        let speaker = browser.regions.tabs[1].speaker.expect("the second tab plays sound");
        press_at(&mut browser, &mut canvas, speaker);
        assert_eq!(browser.take_outputs(), [Output::MuteTab(1)], "a click on the speaker mutes, and does not select");
        // Muted, the speaker stays even once the sound ends.
        browser.set_tabs(
            alloc::vec![
                TabInfo { title: "one".into(), colour: 0, channel: 7, muted: true, private: false },
                TabInfo { title: "two".into(), colour: 1, channel: 8, muted: false, private: false }
            ],
            0,
        );
        browser.set_sounding(BTreeSet::new());
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        assert!(browser.regions.tabs[0].speaker.is_some() && browser.regions.tabs[1].speaker.is_none());
    }

    #[test]
    fn the_mute_all_button_mutes_every_tab_until_all_are_muted() {
        let (mut browser, mut canvas) = browser();
        browser.take_outputs();
        let button = browser.regions.mute_all;
        press_at(&mut browser, &mut canvas, button);
        browser.set_tabs(
            alloc::vec![
                TabInfo { title: "one".into(), colour: 0, channel: 7, muted: true, private: false },
                TabInfo { title: "two".into(), colour: 1, channel: 8, muted: false, private: false }
            ],
            0,
        );
        press_at(&mut browser, &mut canvas, button);
        browser.set_tabs(
            alloc::vec![
                TabInfo { title: "one".into(), colour: 0, channel: 7, muted: true, private: false },
                TabInfo { title: "two".into(), colour: 1, channel: 8, muted: true, private: false }
            ],
            0,
        );
        press_at(&mut browser, &mut canvas, button);
        assert_eq!(browser.take_outputs(), [Output::MuteAll(true), Output::MuteAll(true), Output::MuteAll(false)]);
    }

    #[test]
    fn tabs_back_and_window_buttons_report_what_was_pressed() {
        let (mut browser, mut canvas) = browser();
        browser.take_outputs();
        let TabRegion { tab: second, close, .. } = browser.regions.tabs[1];
        press_at(&mut browser, &mut canvas, Rect::new(second.x0, second.y0, second.x0 + 10.0, second.y1));
        press_at(&mut browser, &mut canvas, close);
        let (back, forward, close_window) = (browser.regions.back, browser.regions.forward, browser.regions.close);
        press_at(&mut browser, &mut canvas, back);
        browser.pointer_released();
        // Forward acts when released, so that holding it opens its list.
        press_at(&mut browser, &mut canvas, forward);
        browser.pointer_released();
        press_at(&mut browser, &mut canvas, close_window);
        assert_eq!(
            browser.take_outputs(),
            [Output::SelectTab(1), Output::CloseTab(1), Output::Forward, Output::Window(WindowAction::Close)],
            "back is disabled, so it reports nothing"
        );
    }

    #[test]
    fn a_click_on_a_page_zooms_it_and_escape_returns() {
        let (mut browser, mut canvas) = browser();
        let (key, rect) = browser.wall().shown().next().unwrap();
        press_at(&mut browser, &mut canvas, rect);
        assert_eq!(browser.wall().zoomed(), Some(key));
        assert!(!browser.take_outputs().iter().any(|output| matches!(output, Output::Cell(_))), "the first click zooms");
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let (_, zoomed) = browser.wall().shown().next().unwrap();
        press_at(&mut browser, &mut canvas, zoomed);
        assert!(browser.take_outputs().iter().any(|output| matches!(output, Output::Cell(_))), "a click on the zoomed page is a cell");
        assert!(browser.key(Key::Escape, Modifiers::default(), 0));
        assert_eq!(browser.wall().zoomed(), None);
    }

    #[test]
    fn the_pointer_is_a_hand_only_over_a_page_a_click_would_zoom() {
        let (mut browser, mut canvas) = browser();
        let (_, rect) = browser.wall().shown().next().unwrap();
        browser.pointer_moved(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1), 0);
        assert_eq!(browser.cursor(), Cursor::Hand);
        let field = browser.regions.field;
        browser.pointer_moved(f64::midpoint(field.x0, field.x1), f64::midpoint(field.y0, field.y1), 0);
        assert_eq!(browser.cursor(), Cursor::Arrow);
        // On a zoomed page a click is on its cells, not a zoom.
        press_at(&mut browser, &mut canvas, rect);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 1_000);
        assert_eq!(browser.cursor(), Cursor::Arrow);
    }

    #[test]
    fn the_wheel_over_the_page_size_bar_slides_it() {
        let (mut browser, mut canvas) = browser();
        let wall = browser.wall();
        let bar = wall.bar_zone_for_tests();
        let before = wall.width_for_tests();
        browser.pointer_moved(f64::midpoint(bar.x0, bar.x1), f64::midpoint(bar.y0, bar.y1), 0);
        // Turned away from the user: larger pages, and the wall not scrolled.
        assert!(browser.scroll_by(-40.0, 0));
        assert!(browser.wall().width_for_tests() > before);
        // Far towards the user: the left edge, the list.
        browser.scroll_by(10_000.0, 0);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 1_000);
        assert_eq!(browser.wall().layout_for_tests(), crate::size::Layout::List);
    }

    /// Requirement 12.1 (decision 63): the middle button closes the tab it
    /// is on, and does nothing elsewhere.
    #[test]
    fn the_middle_button_closes_the_tab_under_it() {
        let (mut browser, mut canvas) = browser();
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        browser.take_outputs();
        let second = browser.regions.tabs[1].tab;
        browser.pointer_moved(f64::midpoint(second.x0, second.x1), f64::midpoint(second.y0, second.y1), 0);
        assert!(browser.middle_pressed());
        assert_eq!(browser.take_outputs(), [Output::CloseTab(1)]);
        browser.pointer_moved(WIDTH / 2.0, HEIGHT / 2.0, 0);
        assert!(!browser.middle_pressed());
        assert_eq!(browser.take_outputs(), [], "not on a tab: nothing");
    }

    /// Shift-Command-W, Control-Shift-W away from macOS, closes the window
    /// (decision 84); Command-W the tab.
    #[test]
    fn shift_w_closes_the_window() {
        let (mut browser, _) = browser();
        browser.take_outputs();
        browser.key(Key::Text("W"), Modifiers { command: true, shift: true, ..Modifiers::default() }, 0);
        assert_eq!(browser.take_outputs(), [Output::Window(WindowAction::Close)]);
        browser.key(Key::Text("w"), Modifiers { command: true, ..Modifiers::default() }, 0);
        assert_eq!(browser.take_outputs(), [Output::CloseTab(browser.active)]);
    }
}
