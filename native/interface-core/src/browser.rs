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
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::canvas::{Canvas, PAGE_HEIGHT, PAGE_WIDTH, PageKey};
use crate::channel::{BANKS, Channel, PAGES, PAGES_PER_BANK, PageInfo, Status, page_index, page_number};
use crate::editor::Editor;
use crate::geometry::{Point, Rect};
use crate::theme::{
    ADDRESS, AXIS, BANK_COLOURS, BANK_COUNT, BANK_NAME, BANK_RANGE, BLACK, BUTTON, CELL_AVAILABLE, CELL_EMPTY, CELL_PULSE, COLUMN_HEAD, Colour, DIM, FIELD,
    FIELD_INK, FIELD_SELECTION, GLYPH, GREEN, HEADING, INSECURE, LEGEND, MUTED, PANEL, RED, ROW_NUMBER, ROW_STATUS, ROW_TITLE, RULE, RULE_SOFT, SECURE,
    SELECTED, STAR_ON, TAB_COLOURS, TAB_TITLE, UPDATED, WHITE, WINDOW, YELLOW, mix,
};
use crate::ui::{HAIRLINE, border, centred, label, left_centred, outline, right_centred};
use crate::wall::{CellClick, UPDATE_GLOW_MILLISECONDS, UPDATE_MARK_MILLISECONDS, Wall, WallPage, glow, since};

#[cfg(feature = "links")]
mod link_ui;
#[cfg(feature = "interaction")]
mod terminal_ui;
#[cfg(feature = "interaction")]
pub use terminal_ui::{CursorView, TypingView};
#[cfg(feature = "phone-keyboard")]
mod phone_ui;
#[cfg(feature = "phone-keyboard")]
pub use phone_ui::{PhoneStatus, PhoneUiView};

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
/// A tab's sound button, before its close button, as Chrome places it.
const TAB_SPEAKER: f64 = 24.0;
/// The speaker glyph's half height; its parts are fractions of it.
const SPEAKER_HALF: f64 = 7.0;
/// The speaker's lines: its sound waves and the cross of a muted one.
const SPEAKER_LINE: f64 = 2.0;
const NEW_TAB: f64 = 38.0;
const STRIP_MARGIN: f64 = 8.0;
const WINDOW_BUTTON_WIDTH: f64 = 44.0;
const WINDOW_BUTTON_HEIGHT: f64 = 32.0;
const WINDOW_BUTTON_GAP: f64 = 6.0;
const WINDOW_GLYPH: f64 = 14.0;
const WINDOW_GLYPH_THICKNESS: f64 = 3.0;

// ---- Toolbar ----
const TOOLBAR_TOP: f64 = STRIP_HEIGHT;
const TOOLBAR_HEIGHT: f64 = 54.0;
const TOOL_SIZE: f64 = 44.0;
const TOOL_INSET: f64 = 5.0;
const FIELD_PADDING: f64 = 12.0;
const LOCK_WIDTH: f64 = 16.0;
const LOCK_HEIGHT: f64 = 12.0;
const SHACKLE_WIDTH: f64 = 10.0;
const SHACKLE_HEIGHT: f64 = 8.0;
const SHACKLE_THICKNESS: f64 = 2.5;
const LOCK_GAP: f64 = 12.0;
const STAR_RADIUS: f64 = 11.0;
const STAR_INNER: f64 = 0.45;
const STAR_POINTS: usize = 5;
const CARET_WIDTH: f64 = 2.0;
const MENU_BAR_WIDTH: f64 = 22.0;
const MENU_BAR_HEIGHT: f64 = 3.0;
const MENU_BAR_GAP: f64 = 7.0;
const ARROW_LENGTH: f64 = 20.0;
const ARROW_HEAD: f64 = 9.0;
const ARROW_THICKNESS: f64 = 3.0;

// ---- Panels ----
const BODY_TOP: f64 = TOOLBAR_TOP + TOOLBAR_HEIGHT + MARGIN;
const MARGIN: f64 = 8.0;
const BANKS_WIDTH: f64 = 212.0;
const LIST_WIDTH: f64 = 260.0;
const OVERVIEW_HEIGHT: f64 = 176.0;
const HEADING_HEIGHT: f64 = 40.0;
const PADDING: f64 = 12.0;
const BANK_GAP: f64 = 6.0;
const BANK_MAX_HEIGHT: f64 = 60.0;
/// Each bank's colour, as a thin bar down the left of its entry.
const SWATCH_WIDTH: f64 = 6.0;
const SWATCH_INSET: f64 = 8.0;
const BANK_BORDER: f64 = 3.0;
const BANK_RANGE_RISE: f64 = 2.0;
const ROW_HEIGHT: f64 = 27.0;
const COLUMN_HEAD_HEIGHT: f64 = 26.0;
/// Where the title column starts: after a three-digit page number.
const TITLE_COLUMN: f64 = 56.0;
const SCROLL_BAR_WIDTH: f64 = 4.0;

// ---- Overview ----
const AXIS_WIDTH: f64 = 40.0;
const CELL_GAP: f64 = 2.0;
const LEGEND_SWATCH: f64 = 10.0;
const LEGEND_GAP: f64 = 6.0;
const LEGEND_SPACING: f64 = 18.0;
/// How long the dot marking a newly received page lasts.
const PULSE_MILLISECONDS: u64 = 900;
/// The dot starts this many times a cell's size and shrinks to nothing.
const PULSE_SCALE: f64 = 2.2;
const PULSE_LIMIT: usize = PAGES;
/// One refresh of a 60 hertz display.
const SMOOTH_FRAME_MILLISECONDS: u64 = 16;
/// Once a second, for the age marks of recent updates.
const MARK_FRAME_MILLISECONDS: u64 = 1_000;
/// An updated row flashes this far towards amber at first.
const ROW_FLASH: f64 = 0.6;
/// The overview's update ring: 1.2 seconds, growing to five cells wide,
/// two pixels thick.
const UPDATE_RING_MILLISECONDS: u64 = 1_200;
const UPDATE_RING_SCALE: f64 = 4.0;
const UPDATE_RING_WIDTH: f64 = 2.0;
/// The update dot leaves a quarter of the cell all round.
const UPDATE_DOT_INSET: f64 = 0.25;
/// Ten frames a second: each dot shrinks away in nine steps.
const PULSE_FRAME_MILLISECONDS: u64 = 100;
/// The button at the overview's top right that folds it into the footer
/// and opens it again, with its chevron.
const TOGGLE_SIZE: f64 = 28.0;
const CHEVRON_WIDTH: f64 = 12.0;
const CHEVRON_HEIGHT: f64 = 6.0;
const CHEVRON_THICKNESS: f64 = 2.5;
/// A page under the pointer in the overview pops up at its native size,
/// this far above its cell, with a shadow offset as the wall's hover.
const PREVIEW_GAP: f64 = 8.0;
const PREVIEW_SHADOW: f64 = 8.0;
const PREVIEW_SHADOW_DARKNESS: f32 = 0.6;

/// Scrolling by one wheel line.
pub const LINE_HEIGHT: f64 = 21.0;
const DECIMAL_DIGITS: usize = 3;

/// How the window's frame is drawn, as Chrome does on each platform: on
/// macOS the system's window buttons sit in the tab strip, so the tabs
/// start after them; elsewhere there is no system frame, and the browser
/// draws its own minimise, maximise and close buttons.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// Where the tabs start: the width the system's window buttons take on macOS.
    pub left_inset: f64,
    /// Whether the browser draws its own minimise, maximise and close buttons.
    pub window_buttons: bool,
}

/// One tab as the trusted core describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabInfo {
    /// The channel's title.
    pub title: String,
    /// An index into [`TAB_COLOURS`].
    pub colour: u8,
    /// The tab's channel, whose sound the interface plays.
    pub channel: u32,
    /// The user muted the tab.
    pub muted: bool,
}

/// What the trusted strip shows. The trusted core decides all of it; the
/// interface only draws it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools, reason = "independent facts the trusted core reports, each drawn on its own")]
pub struct StripState {
    /// The channel's address, as the address field shows it.
    pub address: String,
    /// The channel's host name, highlighted within the address.
    pub identity: String,
    /// The channel was fetched over an encrypted connection.
    pub encrypted: bool,
    /// The address is a file on this computer, not a site.
    pub local: bool,
    /// The tab has history to go back to.
    pub back: bool,
    /// The tab has history to go forward to.
    pub forward: bool,
    /// The channel is bookmarked.
    pub bookmarked: bool,
}

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
    /// Mute the tab at this index, or unmute it.
    MuteTab(usize),
    /// Mute every tab, or unmute every tab.
    MuteAll(bool),
    /// Open the browser's menu.
    Menu,
    /// Something the window should do.
    Window(WindowAction),
    /// Whether text input, including input methods, goes to the address field.
    TextInput(bool),
    /// The pointer or the keyboard's focus is on this cell of a link, or on
    /// no link: the trusted strip shows where it leads.
    #[cfg(feature = "links")]
    Point(Option<crate::links::LinkCell>),
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
    /// A key, text or a click on the open page, whose channel takes input
    /// (`interaction.md` section 5): a key record with no sequence number.
    #[cfg(feature = "interaction")]
    Terminal {
        /// The open page.
        key: PageKey,
        /// The browser's number of the key, for its echo.
        local: u32,
        /// The engine's key record.
        record: Vec<u8>,
    },
    /// Strip input: the phone keyboard's panel opened, or closed.
    #[cfg(feature = "phone-keyboard")]
    PhonePanel(bool),
    /// Strip input: end the phone keyboard's session.
    #[cfg(feature = "phone-keyboard")]
    PhoneFinish,
}

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
pub struct Modifiers {
    /// Shift is held.
    pub shift: bool,
    /// Command (macOS) or Control (elsewhere) is held.
    pub command: bool,
    /// The key is held down and repeating.
    pub repeat: bool,
}

/// Where a tab and its buttons were last drawn.
#[derive(Clone, Copy, Debug)]
struct TabRegion {
    tab: Rect,
    close: Rect,
    /// The sound button, shown while the tab plays sound or is muted.
    speaker: Option<Rect>,
}

/// Where the parts of the window were last drawn.
#[derive(Default)]
struct Regions {
    tabs: Vec<TabRegion>,
    mute_all: Rect,
    new_tab: Rect,
    strip: Rect,
    minimise: Rect,
    maximise: Rect,
    close: Rect,
    back: Rect,
    forward: Rect,
    field: Rect,
    field_text: Rect,
    star: Rect,
    menu: Rect,
    banks: Vec<Rect>,
    list: Rect,
    rows: Vec<(Rect, usize)>,
    overview: Rect,
    overview_toggle: Rect,
    cells: Rect,
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
    /// Hyperlinks (`hyperlinks.md`).
    #[cfg(feature = "links")]
    links: link_ui::LinkUi,
    /// Typing into a page (`interaction.md`).
    #[cfg(feature = "interaction")]
    terminal: terminal_ui::TerminalUi,
    /// The phone keyboard (`interaction.md` section 9).
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

    /// A channel's banks and pages. A different channel starts at its
    /// first bank; the same channel again keeps the choice.
    pub fn set_channel(&mut self, channel: Channel) {
        if channel.channel != self.channel.channel {
            self.versions.clear();
            self.bank = 0;
            self.selected = None;
            self.list_scroll = 0.0;
            self.wall.to_top();
            self.pulses.clear();
            self.updates.clear();
        }
        self.channel = channel;
    }

    /// A page that has just arrived, with its title: it appears in the
    /// banks, the list, the page area and the overview, with the
    /// overview's dot.
    pub fn add_page(&mut self, channel: u32, page: u16, title: String, now: u64) {
        if channel != self.channel.channel {
            return;
        }
        let Some(index) = page_index(page) else { return };
        let Some(slot) = self.channel.pages.get_mut(index) else { return };
        let new = !matches!(slot, Some(info) if info.status == Status::Loaded);
        // A folded overview keeps no animation; opened, it draws the page
        // as it is.
        if new && !self.overview_folded && self.pulses.len() < PULSE_LIMIT {
            self.pulses.push((index, now));
        }
        *slot = Some(PageInfo { title, status: Status::Loaded });
        #[cfg(feature = "links")]
        self.show_pending(channel, page, now);
    }

    /// A page has a new version: it glows, and is marked with its age.
    pub fn page_updated(&mut self, channel: u32, page: u16, now: u64) {
        if channel != self.channel.channel {
            return;
        }
        let Some(index) = page_index(page) else { return };
        self.updates.retain(|&(updated, _)| updated != index);
        self.updates.push((index, now));
    }

    fn updated_at(&self, index: usize) -> Option<u64> {
        self.updates.iter().find(|&&(updated, _)| updated == index).map(|&(_, at)| at)
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

    /// Whether anything is moving at `now`, so the host should keep drawing.
    #[must_use]
    pub fn animating(&self, now: u64) -> bool {
        self.frame_interval(now).is_some()
    }

    /// How often to draw while something moves, in milliseconds: every
    /// display refresh for the page area's transitions, and ten times a
    /// second for the overview's dots, which need no more.
    #[must_use]
    pub fn frame_interval(&self, now: u64) -> Option<u64> {
        let since_update = |limit: u64| self.updates.iter().any(|&(_, at)| now.saturating_sub(at) < limit);
        if self.wall.animating(now) {
            Some(SMOOTH_FRAME_MILLISECONDS)
        } else if self.pulses.iter().any(|&(_, start)| now.saturating_sub(start) < PULSE_MILLISECONDS) || since_update(UPDATE_GLOW_MILLISECONDS) {
            Some(PULSE_FRAME_MILLISECONDS)
        } else if since_update(UPDATE_MARK_MILLISECONDS) {
            // The "Updated 12s" marks count whole seconds.
            Some(MARK_FRAME_MILLISECONDS)
        } else {
            #[cfg(feature = "interaction")]
            return self.terminal_frame_interval();
            #[cfg(not(feature = "interaction"))]
            None
        }
    }

    /// Advances what waits on time; returns whether anything changed.
    pub fn settle(&mut self, now: u64) -> bool {
        let before = self.pulses.len();
        self.pulses.retain(|&(_, start)| now.saturating_sub(start) < PULSE_MILLISECONDS);
        let updates = self.updates.len();
        self.updates.retain(|&(_, at)| now.saturating_sub(at) < UPDATE_MARK_MILLISECONDS);
        #[cfg(feature = "interaction")]
        let echoes = self.settle_echoes(now);
        #[cfg(not(feature = "interaction"))]
        let echoes = false;
        self.pulses.len() != before || self.updates.len() != updates || echoes
    }

    fn page_key(&self, index: usize) -> Option<PageKey> {
        page_number(index).map(|page| PageKey { channel: self.channel.channel, page })
    }

    fn select_page(&mut self, index: usize, reveal: bool) {
        let bank = index / PAGES_PER_BANK;
        if bank != self.bank {
            self.choose_bank(bank);
        }
        self.selected = Some(index);
        if reveal && let Some(key) = self.page_key(index) {
            self.wall.reveal(key);
            // A zoomed area shows the newly chosen page instead.
            if self.wall.zoomed().is_some() {
                self.wall.switch_zoom(key);
            }
        }
        self.reveal_row(index);
    }

    fn choose_bank(&mut self, bank: usize) {
        self.bank = bank;
        self.selected = None;
        self.list_scroll = 0.0;
        self.wall.to_top();
    }

    /// Scrolls the list so that page `index`'s row is in view.
    fn reveal_row(&mut self, index: usize) {
        let position = self.bank_pages().iter().position(|&candidate| candidate == index);
        let Some(position) = position else { return };
        #[allow(clippy::cast_precision_loss, reason = "at most 100 rows")]
        let top = position as f64 * ROW_HEIGHT;
        let view = self.regions.list.height();
        if top < self.list_scroll {
            self.list_scroll = top;
        } else if top + ROW_HEIGHT > self.list_scroll + view {
            self.list_scroll = top + ROW_HEIGHT - view;
        }
    }

    /// The indices of the chosen bank's pages.
    fn bank_pages(&self) -> Vec<usize> {
        let first = self.bank * PAGES_PER_BANK;
        (first..first + PAGES_PER_BANK).filter(|&index| matches!(self.channel.pages.get(index), Some(Some(_)))).collect()
    }

    fn stop_editing(&mut self) {
        if self.editing {
            self.editing = false;
            self.outputs.push(Output::TextInput(false));
        }
    }

    fn start_editing(&mut self) {
        if !self.editing {
            self.editing = true;
            self.editor.set(&self.strip.address);
            self.field_scroll = 0.0;
            self.outputs.push(Output::TextInput(true));
        }
    }

    // ---- Input ----

    /// The pointer moved to `x`, `y` in logical pixels; returns whether
    /// the window needs drawing again.
    pub fn pointer_moved(&mut self, x: f64, y: f64, now: u64) -> bool {
        self.pointer = Point::new(x, y);
        let dragged = self.wall.pointer_moved(self.pointer, now);
        let hovered = self.wall.hover(self.pointer, now);
        let previewed = self.overview_page(self.pointer);
        let preview_changed = previewed != self.previewed;
        self.previewed = previewed;
        #[cfg(feature = "links")]
        let linked = self.repoint();
        #[cfg(not(feature = "links"))]
        let linked = false;
        dragged || hovered || preview_changed || linked
    }

    /// The overview's cell under `point`, when its page has arrived.
    fn overview_page(&self, point: Point) -> Option<usize> {
        let index = self.cell_index(point)?;
        matches!(self.channel.pages.get(index), Some(Some(info)) if info.status == Status::Loaded).then_some(index)
    }

    /// The page the user has opened, whose sounds play (the engine's
    /// REQ-AUD-012): the zoomed page, or else the page chosen by number or
    /// in the list, as a television goes to the page whose number is typed.
    #[must_use]
    pub fn opened(&self) -> Option<PageKey> {
        self.wall.zoomed().or_else(|| self.selected.and_then(|index| self.page_key(index)))
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

    /// Folds the overview into the footer, or opens it again.
    fn toggle_overview(&mut self) {
        self.overview_folded = !self.overview_folded;
        self.pulses.clear();
        self.previewed = None;
    }

    /// The main button was released.
    pub fn pointer_released(&mut self) {
        self.wall.pointer_released();
    }

    /// A press of the main button, with `extend` when Shift is held.
    #[allow(clippy::too_many_lines, reason = "one hit-test per part of the window, in drawing order")]
    pub fn pointer_pressed<C: Canvas + ?Sized>(&mut self, canvas: &mut C, extend: bool, now: u64) -> bool {
        #[cfg(feature = "phone-keyboard")]
        if self.phone_press(self.pointer) {
            return true;
        }
        // The strip's question, drawn over the address field.
        #[cfg(feature = "links")]
        if self.link_press(now) {
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
        if let Some(index) = regions.tabs.iter().position(|region| region.tab.contains(point)) {
            let region = regions.tabs[index];
            self.outputs.push(if region.close.contains(point) {
                Output::CloseTab(index)
            } else if region.speaker.is_some_and(|speaker| speaker.contains(point)) {
                Output::MuteTab(index)
            } else {
                Output::SelectTab(index)
            });
            return true;
        }
        let mute_all = Output::MuteAll(!self.all_muted());
        let action = [
            (regions.new_tab, Output::NewTab),
            (regions.back, Output::Back),
            (regions.forward, Output::Forward),
            (regions.star, Output::Bookmark),
            (regions.mute_all, mute_all),
            (regions.menu, Output::Menu),
            (regions.minimise, Output::Window(WindowAction::Minimise)),
            (regions.maximise, Output::Window(WindowAction::Maximise)),
            (regions.close, Output::Window(WindowAction::Close)),
        ]
        .into_iter()
        .find(|(rect, _)| rect.contains(point));
        if let Some((_, output)) = action {
            // The menu holds the phone keyboard.
            #[cfg(feature = "phone-keyboard")]
            if output == Output::Menu {
                self.toggle_phone_panel();
                return true;
            }
            let allowed = match output {
                Output::Back => self.strip.back,
                Output::Forward => self.strip.forward,
                _ => true,
            };
            if allowed {
                self.outputs.push(output);
            }
            return true;
        }
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
            // On a page whose channel takes input a plain click is the
            // host's, as a terminal with mouse reporting gives it; the
            // browser's modifier makes it the browser's (`interaction.md`
            // section 5).
            #[cfg(feature = "interaction")]
            if self.wall.zoomed().is_some() && self.click_is_hosts() {
                #[allow(clippy::cast_possible_truncation, reason = "the record's clock wraps")]
                return self.terminal_click(now as u32);
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

    /// Scrolls whatever is under the pointer.
    pub fn scroll_by(&mut self, distance: f64, now: u64) -> bool {
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
            return self.shortcut(key, now);
        }
        if self.editing {
            return self.edit(key, modifiers.shift);
        }
        #[cfg(feature = "links")]
        if let Some(changed) = self.link_key(key, modifiers, now) {
            return changed;
        }
        match key {
            Key::Escape => return self.wall.unzoom(now),
            Key::Left => self.wall.step(-1, now),
            Key::Right => self.wall.step(1, now),
            Key::Up | Key::Down => self.step_page(key == Key::Down, now),
            Key::Text(text) => {
                // Three digits choose a page, as on a television.
                for digit in text.chars().filter(char::is_ascii_digit) {
                    self.digits.push(digit);
                    if self.digits.len() == DECIMAL_DIGITS {
                        let page = u16::from_str_radix(&self.digits, 16).ok();
                        self.digits.clear();
                        // A number the open page or the channel links
                        // elsewhere goes to the trusted core.
                        #[cfg(feature = "links")]
                        if page.is_some_and(|page| self.link_typed(page)) {
                            continue;
                        }
                        if let Some(index) = page.and_then(page_index)
                            && matches!(self.channel.pages.get(index), Some(Some(_)))
                        {
                            self.select_page(index, true);
                        }
                    }
                }
            }
            _ => return false,
        }
        true
    }

    fn shortcut(&mut self, key: Key<'_>, now: u64) -> bool {
        // The browser's Escape: leaves a page whose channel has every
        // other key (`interaction.md` section 5).
        if key == Key::Escape {
            self.stop_editing();
            return self.wall.unzoom(now);
        }
        let Key::Text(text) = key else { return false };
        match text.to_ascii_lowercase().as_str() {
            "l" => self.start_editing(),
            "a" if self.editing => self.editor.select_all(),
            "t" => self.outputs.push(Output::NewTab),
            "w" => self.outputs.push(Output::CloseTab(self.active)),
            "[" if self.strip.back => self.outputs.push(Output::Back),
            "]" if self.strip.forward => self.outputs.push(Output::Forward),
            "d" => self.outputs.push(Output::Bookmark),
            _ => return false,
        }
        true
    }

    fn edit(&mut self, key: Key<'_>, shift: bool) -> bool {
        match key {
            Key::Text(text) => self.editor.insert(text),
            Key::Enter => {
                let address = String::from(self.editor.text().trim());
                self.stop_editing();
                if !address.is_empty() {
                    self.outputs.push(Output::Navigate(address));
                }
            }
            Key::Escape => self.stop_editing(),
            Key::Tab => return false,
            Key::Backspace => self.editor.backspace(),
            Key::Delete => self.editor.delete(),
            Key::Left => self.editor.left(shift),
            Key::Right => self.editor.right(shift),
            Key::Home | Key::Up => self.editor.home(shift),
            Key::End | Key::Down => self.editor.end(shift),
        }
        true
    }

    /// The list and the overview follow the zoomed page.
    fn follow_zoom(&mut self) {
        if let Some(index) = self.wall.zoomed().and_then(|key| page_index(key.page)) {
            self.selected = Some(index);
            self.reveal_row(index);
        }
    }

    fn step_page(&mut self, forward: bool, now: u64) {
        if self.wall.zoomed().is_some() {
            if self.wall.step_zoom(if forward { 1 } else { -1 }, now).is_some() {
                self.follow_zoom();
            }
            return;
        }
        let pages = self.bank_pages();
        let position = self.selected.and_then(|selected| pages.iter().position(|&index| index == selected));
        let next = match (position, forward) {
            (None, _) => pages.first(),
            (Some(position), true) => pages.get(position + 1),
            (Some(position), false) => position.checked_sub(1).and_then(|previous| pages.get(previous)),
        };
        if let Some(&index) = next {
            self.select_page(index, true);
        }
    }

    /// The byte offset in the address nearest to `x`.
    fn offset_at<C: Canvas + ?Sized>(&self, canvas: &mut C, x: f64) -> usize {
        let text = self.editor.text();
        let target = x - self.regions.field_text.x0 + self.field_scroll;
        let mut best = (0, f64::MAX);
        for (offset, _) in text.char_indices().chain(core::iter::once((text.len(), ' '))) {
            let distance = (canvas.text_width(&text[..offset], ADDRESS) - target).abs();
            if distance < best.1 {
                best = (offset, distance);
            }
        }
        best.0
    }

    fn cell_index(&self, point: Point) -> Option<usize> {
        let cells = self.regions.cells;
        if !cells.contains(point) {
            return None;
        }
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let (width, height) = (cells.width() / PAGES_PER_BANK as f64, cells.height() / BANKS as f64);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "the point is inside the grid")]
        let (column, row) = (((point.x - cells.x0) / width) as usize, ((point.y - cells.y0) / height) as usize);
        (column < PAGES_PER_BANK && row < BANKS).then_some(row * PAGES_PER_BANK + column)
    }

    // ---- Drawing ----

    /// Draws the whole window; `width` and `height` are logical pixels.
    pub fn draw<C: Canvas + ?Sized>(&mut self, canvas: &mut C, width: f64, height: f64, now: u64) {
        self.regions = Regions::default();
        canvas.fill(Rect::new(0.0, 0.0, width, height), 0.0, WINDOW);
        let overview_height = if self.overview_folded { HEADING_HEIGHT } else { OVERVIEW_HEIGHT };
        let overview = Rect::new(MARGIN, height - MARGIN - overview_height, width - MARGIN, height - MARGIN);
        let body_bottom = overview.y0 - MARGIN;
        let banks = Rect::new(MARGIN, BODY_TOP, MARGIN + BANKS_WIDTH, body_bottom);
        let list = Rect::new(banks.x1 + MARGIN, BODY_TOP, banks.x1 + MARGIN + LIST_WIDTH, body_bottom);
        let zoom = Rect::new(list.x1 + MARGIN, BODY_TOP, width - MARGIN, body_bottom);

        let pages: Vec<WallPage> = self
            .bank_pages()
            .into_iter()
            .filter_map(|index| {
                let info = self.channel.pages.get(index)?.as_ref()?;
                Some(WallPage { key: self.page_key(index)?, title: info.title.clone(), updated: self.updated_at(index) })
            })
            .collect();
        let selected = self.selected.and_then(|index| self.page_key(index));
        self.wall.draw(canvas, zoom, &pages, selected, now);
        #[cfg(feature = "links")]
        self.draw_links(canvas);
        #[cfg(feature = "interaction")]
        self.draw_terminal(canvas, now);
        let mut visible = self.wall.visible();
        // The overview's previewed page is on screen too, so the trusted
        // core has it rendered, whichever bank it is in.
        if let Some(key) = self.previewed.and_then(|index| self.page_key(index))
            && !visible.contains(&key)
        {
            visible.push(key);
        }
        if visible != self.visible {
            self.visible.clone_from(&visible);
            self.outputs.push(Output::Visible(visible));
        }
        self.draw_banks(canvas, banks);
        self.draw_list(canvas, list, now);
        self.draw_overview(canvas, overview, now);
        self.draw_toolbar(canvas, width);
        self.draw_tab_strip(canvas, width);
        self.draw_preview(canvas, width);
        #[cfg(feature = "phone-keyboard")]
        self.draw_phone_panel(canvas);
        // Pages may have moved under a still pointer: by scrolling, a new
        // size or a zoom.
        self.wall.hover(self.pointer, now);
    }

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
        #[allow(clippy::cast_precision_loss, reason = "a few tabs")]
        let count = self.tabs.len().max(1) as f64;
        let room = right - left - NEW_TAB - TAB_GAP * 2.0;
        let tab_width = ((room - TAB_GAP * (count - 1.0)) / count).clamp(TAB_MIN_WIDTH, TAB_MAX_WIDTH);
        let mut x = left;
        for (index, tab) in self.tabs.iter().enumerate() {
            if x + tab_width > right - NEW_TAB {
                break;
            }
            let active = index == self.active;
            let rect = Rect::new(x, if active { TAB_TOP } else { TAB_TOP_INACTIVE }, x + tab_width, STRIP_HEIGHT);
            let (fill, ink) = TAB_COLOURS[usize::from(tab.colour) % TAB_COLOURS.len()];
            canvas.fill(rect, 0.0, fill);
            if active {
                border(canvas, Rect::new(rect.x0, rect.y0, rect.x1, rect.y1 + BANK_BORDER), BANK_BORDER - 1.0, WHITE);
            }
            let close = Rect::new(rect.x1 - TAB_CLOSE - TAB_GAP * 2.0, rect.y0, rect.x1 - TAB_GAP, rect.y1);
            let speaker = (tab.muted || self.sounding.contains(&tab.channel)).then(|| Rect::new(close.x0 - TAB_SPEAKER, rect.y0, close.x0, rect.y1));
            canvas.push_clip(Rect::new(rect.x0, rect.y0, speaker.map_or(close.x0, |speaker| speaker.x0), rect.y1));
            left_centred(canvas, rect, TAB_TEXT_INSET, &tab.title, TAB_TITLE, ink);
            canvas.pop_clip();
            if let Some(speaker) = speaker {
                speaker_glyph(canvas, speaker, tab.muted, ink);
            }
            centred(canvas, close, "×", GLYPH, ink);
            self.regions.tabs.push(TabRegion { tab: rect, close, speaker });
            x = rect.x1 + TAB_GAP;
        }
        self.regions.new_tab = Rect::new(x + TAB_GAP, TAB_TOP_INACTIVE, x + TAB_GAP + NEW_TAB, STRIP_HEIGHT - TAB_GAP);
        canvas.fill(self.regions.new_tab, 0.0, BUTTON);
        outline(canvas, self.regions.new_tab, RULE);
        centred(canvas, self.regions.new_tab, "+", GLYPH, WHITE);
        self.regions.strip = Rect::new(0.0, 0.0, width, STRIP_HEIGHT);
    }

    fn draw_toolbar<C: Canvas + ?Sized>(&mut self, canvas: &mut C, width: f64) {
        let bar = Rect::new(0.0, TOOLBAR_TOP, width, TOOLBAR_TOP + TOOLBAR_HEIGHT);
        canvas.fill(bar, 0.0, BLACK);
        canvas.fill(Rect::new(0.0, bar.y1 - HAIRLINE, width, bar.y1), 0.0, RULE);
        let top = bar.y0 + TOOL_INSET;
        let tool = |x: f64| Rect::new(x, top, x + TOOL_SIZE, top + TOOL_SIZE);
        self.regions.back = tool(MARGIN);
        self.regions.forward = tool(MARGIN + TOOL_SIZE);
        for (rect, enabled, pointing_left) in [(self.regions.back, self.strip.back, true), (self.regions.forward, self.strip.forward, false)] {
            canvas.fill(rect, 0.0, BUTTON);
            border(canvas, rect, 2.0, if enabled { SELECTED } else { RULE });
            arrow(canvas, rect, pointing_left, if enabled { WHITE } else { DIM });
        }
        self.regions.menu = tool(width - MARGIN - TOOL_SIZE);
        // Mute or unmute every tab, beside the menu.
        self.regions.mute_all = tool(self.regions.menu.x0 - MARGIN - TOOL_SIZE);
        canvas.fill(self.regions.mute_all, 0.0, BUTTON);
        border(canvas, self.regions.mute_all, 2.0, RULE);
        speaker_glyph(canvas, self.regions.mute_all, self.all_muted(), WHITE);
        canvas.fill(self.regions.menu, 0.0, BUTTON);
        border(canvas, self.regions.menu, 2.0, RULE);
        let middle = f64::midpoint(self.regions.menu.y0, self.regions.menu.y1);
        let centre_x = f64::midpoint(self.regions.menu.x0, self.regions.menu.x1);
        for offset in [-MENU_BAR_GAP, 0.0, MENU_BAR_GAP] {
            let y = middle + offset - MENU_BAR_HEIGHT / 2.0;
            canvas.fill(Rect::new(centre_x - MENU_BAR_WIDTH / 2.0, y, centre_x + MENU_BAR_WIDTH / 2.0, y + MENU_BAR_HEIGHT), 0.0, WHITE);
        }

        let field = Rect::new(self.regions.forward.x1 + MARGIN * 2.0, top, self.regions.mute_all.x0 - MARGIN * 2.0, top + TOOL_SIZE);
        self.regions.field = field;
        canvas.fill(field, 0.0, FIELD);
        if self.editing {
            border(canvas, field, 2.0, SELECTED);
        }
        // The strip's question takes the whole field: no address, no star.
        #[cfg(feature = "links")]
        if self.draw_prompt(canvas, field) {
            self.regions.star = Rect::ZERO;
            self.regions.field_text = Rect::ZERO;
            return;
        }
        let middle = f64::midpoint(field.y0, field.y1);
        let mut text_left = field.x0 + FIELD_PADDING;
        if self.strip.local {
            let word = "File";
            left_centred(canvas, Rect::new(text_left, field.y0, field.x1, field.y1), 0.0, word, ROW_STATUS, FIELD_INK);
            text_left += canvas.text_width(word, ROW_STATUS) + LOCK_GAP;
        } else if self.strip.encrypted {
            padlock(canvas, Point::new(text_left, middle), SECURE);
            text_left += LOCK_WIDTH + LOCK_GAP;
        } else {
            let warning = "Not secure";
            left_centred(canvas, Rect::new(text_left, field.y0, field.x1, field.y1), 0.0, warning, ROW_STATUS, INSECURE);
            text_left += canvas.text_width(warning, ROW_STATUS) + LOCK_GAP;
        }
        self.regions.star = Rect::new(field.x1 - TOOL_SIZE, field.y0, field.x1, field.y1);
        star(canvas, Point::new(f64::midpoint(self.regions.star.x0, self.regions.star.x1), middle), if self.strip.bookmarked { STAR_ON } else { FIELD_INK });
        let text = Rect::new(text_left, field.y0, self.regions.star.x0, field.y1);
        #[cfg(feature = "phone-keyboard")]
        let text = self.draw_phone_badge(canvas, text);
        #[cfg(feature = "interaction")]
        let text = self.draw_typing_badge(canvas, text);
        self.regions.field_text = text;
        // While the pointer or the focus is on a link, the field shows where
        // it leads (requirement 17.6).
        #[cfg(feature = "links")]
        let pointing = !self.editing && self.draw_pointing(canvas, text);
        #[cfg(not(feature = "links"))]
        let pointing = false;
        if !pointing {
            self.draw_address(canvas, text);
        }
    }

    fn draw_address<C: Canvas + ?Sized>(&mut self, canvas: &mut C, area: Rect) {
        let line = canvas.line_height(ADDRESS);
        let y = (area.y0 + area.y1 - line) / 2.0;
        canvas.push_clip(area);
        if self.editing {
            let text = self.editor.text();
            let caret = canvas.text_width(&text[..self.editor.caret()], ADDRESS);
            // Keep the caret in view.
            let room = area.width() - CARET_WIDTH;
            if caret - self.field_scroll > room {
                self.field_scroll = caret - room;
            } else if caret < self.field_scroll {
                self.field_scroll = caret;
            }
            let x = area.x0 - self.field_scroll;
            let (start, end) = self.editor.selection();
            if start != end {
                let from = canvas.text_width(&text[..start], ADDRESS);
                let to = canvas.text_width(&text[..end], ADDRESS);
                canvas.fill(Rect::new(x + from, y, x + to, y + line), 0.0, FIELD_SELECTION);
            }
            canvas.text(Rect::new(x, y, area.x1 + self.field_scroll, y + line), text, ADDRESS, FIELD_INK);
            canvas.fill(Rect::new(x + caret, y, x + caret + CARET_WIDTH, y + line), 0.0, FIELD_INK);
        } else {
            // The address as the trusted core formatted it, its host in
            // full ink and the rest muted, as browsers show it.
            let address = &self.strip.address;
            let identity_at = address.find(self.strip.identity.as_str()).filter(|_| !self.strip.identity.is_empty());
            let mut x = area.x0;
            let parts: [(&str, Colour); 3] = match identity_at {
                Some(start) => {
                    let end = start + self.strip.identity.len();
                    [(&address[..start], DIM), (&address[start..end], FIELD_INK), (&address[end..], DIM)]
                }
                None => [(address.as_str(), FIELD_INK), ("", DIM), ("", DIM)],
            };
            for (part, colour) in parts {
                canvas.text(Rect::new(x, y, area.x1, y + line), part, ADDRESS, colour);
                x += canvas.text_width(part, ADDRESS);
            }
        }
        canvas.pop_clip();
    }

    fn draw_banks<C: Canvas + ?Sized>(&mut self, canvas: &mut C, panel: Rect) {
        canvas.fill(panel, 0.0, PANEL);
        outline(canvas, panel, RULE);
        let heading = Rect::new(panel.x0, panel.y0, panel.x1, panel.y0 + HEADING_HEIGHT);
        left_centred(canvas, heading, PADDING, "Page banks", HEADING, WHITE);
        canvas.fill(Rect::new(panel.x0, heading.y1 - HAIRLINE, panel.x1, heading.y1), 0.0, RULE);
        #[allow(clippy::cast_precision_loss, reason = "eight banks")]
        let height = ((panel.height() - HEADING_HEIGHT - BANK_GAP) / BANKS as f64 - BANK_GAP).min(BANK_MAX_HEIGHT);
        for (bank, &colour) in BANK_COLOURS.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "eight banks")]
            let top = heading.y1 + BANK_GAP + bank as f64 * (height + BANK_GAP);
            let rect = Rect::new(panel.x0 + BANK_GAP, top, panel.x1 - BANK_GAP, top + height);
            let chosen = bank == self.bank;
            canvas.fill(rect, 0.0, if chosen { BUTTON } else { PANEL });
            if chosen {
                border(canvas, rect, BANK_BORDER, colour);
            } else {
                outline(canvas, rect, RULE_SOFT);
            }
            let middle = f64::midpoint(rect.y0, rect.y1);
            let swatch = Rect::new(rect.x0 + PADDING, rect.y0 + SWATCH_INSET, rect.x0 + PADDING + SWATCH_WIDTH, rect.y1 - SWATCH_INSET);
            canvas.fill(swatch, 0.0, colour);
            let text_left = swatch.x1 + PADDING;
            let range = format!("{}00-{}99", bank + 1, bank + 1);
            let name = self.channel.banks.get(bank).map_or("", String::as_str);
            let range_height = canvas.line_height(BANK_RANGE);
            let name_height = canvas.line_height(BANK_NAME);
            let top = middle - f64::midpoint(range_height, name_height) - BANK_RANGE_RISE;
            label(canvas, Point::new(text_left, top), &range, BANK_RANGE, colour);
            canvas.text(Rect::new(text_left, top + range_height, rect.x1 - PADDING, top + range_height + name_height), name, BANK_NAME, colour);
            right_centred(
                canvas,
                Rect::new(rect.x0, rect.y0, rect.x1, rect.y0 + range_height + PADDING),
                PADDING,
                &format!("{}", self.channel.bank_count(bank)),
                BANK_COUNT,
                MUTED,
            );
            self.regions.banks.push(rect);
        }
    }

    fn draw_list<C: Canvas + ?Sized>(&mut self, canvas: &mut C, panel: Rect, now: u64) {
        canvas.fill(panel, 0.0, PANEL);
        let heading = Rect::new(panel.x0, panel.y0, panel.x1, panel.y0 + HEADING_HEIGHT);
        canvas.fill(heading, 0.0, SELECTED);
        let pages = self.bank_pages();
        let first = self.bank + 1;
        let name = self.channel.banks.get(self.bank).map_or("", String::as_str);
        let title = if name.is_empty() {
            format!("Pages {first}00-{first}99 ({})", pages.len())
        } else {
            format!("{first}00 {name} · pages {first}00-{first}99 ({})", pages.len())
        };
        canvas.push_clip(heading);
        left_centred(canvas, heading, PADDING, &title, HEADING, WHITE);
        canvas.pop_clip();

        let head = Rect::new(panel.x0, heading.y1, panel.x1, heading.y1 + COLUMN_HEAD_HEIGHT);
        let title_x = panel.x0 + TITLE_COLUMN;
        for (x, text) in [(panel.x0 + PADDING, "Page"), (title_x + PADDING, "Title")] {
            left_centred(canvas, Rect::new(x, head.y0, panel.x1, head.y1), 0.0, text, COLUMN_HEAD, MUTED);
        }
        canvas.fill(Rect::new(panel.x0, head.y1 - HAIRLINE, panel.x1, head.y1), 0.0, RULE);

        let area = Rect::new(panel.x0, head.y1, panel.x1, panel.y1);
        self.regions.list = area;
        #[allow(clippy::cast_precision_loss, reason = "at most 100 rows")]
        let content = pages.len() as f64 * ROW_HEIGHT;
        let max_scroll = (content - area.height()).max(0.0);
        self.list_scroll = self.list_scroll.min(max_scroll);
        canvas.push_clip(area);
        for (position, &index) in pages.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "at most 100 rows")]
            let top = area.y0 + position as f64 * ROW_HEIGHT - self.list_scroll;
            if top > area.y1 || top + ROW_HEIGHT < area.y0 {
                continue;
            }
            let row = Rect::new(area.x0, top, area.x1, top + ROW_HEIGHT);
            self.regions.rows.push((row, index));
            let Some(Some(info)) = self.channel.pages.get(index) else {
                continue;
            };
            let chosen = self.selected == Some(index);
            if chosen {
                canvas.fill(row, 0.0, SELECTED);
            }
            canvas.fill(Rect::new(row.x0, row.y1 - HAIRLINE, row.x1, row.y1), 0.0, RULE_SOFT);
            // A row just updated flashes amber and fades back.
            let updated = self.updated_at(index);
            let glowing = glow(updated, now);
            if glowing > 0.0 {
                canvas.fill(row, 0.0, mix(if chosen { SELECTED } else { PANEL }, UPDATED, glowing * ROW_FLASH));
            }
            let number = page_number(index).map_or_else(String::new, |page| format!("{page:03X}"));
            left_centred(canvas, row, PADDING, &number, ROW_NUMBER, WHITE);
            let mut title_end = row.x1 - PADDING;
            if let Some(age) = since(updated, now) {
                title_end -= canvas.text_width(&age, ROW_STATUS) + PADDING;
                right_centred(canvas, row, PADDING, &age, ROW_STATUS, UPDATED);
            }
            canvas.push_clip(Rect::new(title_x, row.y0, title_end, row.y1));
            left_centred(canvas, Rect::new(title_x, row.y0, row.x1, row.y1), PADDING, &info.title, ROW_TITLE, WHITE);
            canvas.pop_clip();
        }
        canvas.pop_clip();
        if max_scroll > 0.0 {
            let bar = area.height() * area.height() / content;
            let top = area.y0 + (area.height() - bar) * self.list_scroll / max_scroll;
            canvas.fill(Rect::new(area.x1 - SCROLL_BAR_WIDTH * 2.0, top, area.x1 - SCROLL_BAR_WIDTH, top + bar), SCROLL_BAR_WIDTH / 2.0, RULE);
        }
        outline(canvas, panel, RULE);
    }

    /// The overview's legend and page counts in `heading`, right-aligned to
    /// end before `right`.
    fn draw_legend<C: Canvas + ?Sized>(&self, canvas: &mut C, heading: Rect, right: f64) {
        let middle = f64::midpoint(heading.y0, heading.y1);
        let (present, loaded) = self.channel.counts();
        let mut x = right - LEGEND_SPACING;
        let counts = format!("{present} of {PAGES} pages have content, {loaded} loaded");
        x -= canvas.text_width(&counts, LEGEND);
        left_centred(canvas, Rect::new(x, heading.y0, heading.x1, heading.y1), 0.0, &counts, LEGEND, MUTED);
        x -= LEGEND_SPACING * 2.0;
        for (word, colour, ring) in [
            ("Empty", CELL_EMPTY, false),
            ("Available", CELL_AVAILABLE, false),
            ("Loaded", YELLOW, false),
            ("Shown", YELLOW, true),
            ("Updated", UPDATED, false),
        ]
        .into_iter()
        .rev()
        {
            let width = canvas.text_width(word, LEGEND);
            x -= width;
            left_centred(canvas, Rect::new(x, heading.y0, x + width, heading.y1), 0.0, word, LEGEND, MUTED);
            x -= LEGEND_GAP + LEGEND_SWATCH;
            let swatch = Rect::new(x, middle - LEGEND_SWATCH / 2.0, x + LEGEND_SWATCH, middle + LEGEND_SWATCH / 2.0);
            canvas.fill(swatch, 0.0, colour);
            if ring {
                outline(canvas, swatch, WHITE);
            }
            x -= LEGEND_SPACING;
        }
    }

    fn draw_overview<C: Canvas + ?Sized>(&mut self, canvas: &mut C, panel: Rect, now: u64) {
        canvas.fill(panel, 0.0, PANEL);
        outline(canvas, panel, RULE);
        let heading = Rect::new(panel.x0, panel.y0, panel.x1, panel.y0 + HEADING_HEIGHT);
        left_centred(canvas, heading, PADDING, &format!("All {PAGES} pages · overview"), HEADING, WHITE);
        let middle = f64::midpoint(heading.y0, heading.y1);
        let toggle = Rect::new(heading.x1 - PADDING - TOGGLE_SIZE, middle - TOGGLE_SIZE / 2.0, heading.x1 - PADDING, middle + TOGGLE_SIZE / 2.0);
        self.regions.overview = panel;
        self.regions.overview_toggle = toggle;
        canvas.fill(toggle, 0.0, BUTTON);
        outline(canvas, toggle, RULE);
        // Down folds it away; up brings it back.
        chevron(canvas, toggle, !self.overview_folded, WHITE);
        if self.overview_folded {
            return;
        }
        self.draw_legend(canvas, heading, toggle.x0);

        let cells = Rect::new(panel.x0 + PADDING + AXIS_WIDTH, heading.y1, panel.x1 - PADDING, panel.y1 - PADDING);
        self.regions.cells = cells;
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let (width, height) = (cells.width() / PAGES_PER_BANK as f64, cells.height() / BANKS as f64);
        for (bank, &bank_colour) in BANK_COLOURS.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "eight banks")]
            let top = cells.y0 + bank as f64 * height;
            let axis = Rect::new(panel.x0 + PADDING, top, cells.x0, top + height);
            left_centred(canvas, axis, 0.0, &format!("{}00", bank + 1), AXIS, if bank == self.bank { WHITE } else { MUTED });
            if bank == self.bank {
                canvas.fill(Rect::new(axis.x0 - PADDING / 2.0, top + CELL_GAP, axis.x0 - PADDING / 2.0 + CELL_GAP, top + height - CELL_GAP), 0.0, WHITE);
            }
            for column in 0..PAGES_PER_BANK {
                let index = bank * PAGES_PER_BANK + column;
                #[allow(clippy::cast_precision_loss, reason = "small counts")]
                let left = cells.x0 + column as f64 * width;
                let cell = Rect::new(left, top, left + width - CELL_GAP, top + height - CELL_GAP);
                let colour = match self.channel.pages.get(index) {
                    Some(Some(info)) if info.status == Status::Loaded => bank_colour,
                    Some(Some(_)) => CELL_AVAILABLE,
                    _ => CELL_EMPTY,
                };
                canvas.fill(cell, 0.0, colour);
                if self.page_key(index).is_some_and(|key| self.visible.contains(&key)) {
                    outline(canvas, cell, WHITE);
                }
                if self.selected == Some(index) {
                    border(canvas, Rect::new(cell.x0 - CELL_GAP, cell.y0 - CELL_GAP, cell.x1 + CELL_GAP, cell.y1 + CELL_GAP), CELL_GAP, SELECTED);
                }
                // An amber dot for a minute after each update.
                if since(self.updated_at(index), now).is_some() {
                    let inset = cell.width().min(cell.height()) * UPDATE_DOT_INSET;
                    let dot = Rect::new(cell.x0 + inset, cell.y0 + inset, cell.x1 - inset, cell.y1 - inset);
                    canvas.fill(dot, dot.width().min(dot.height()) / 2.0, UPDATED);
                }
            }
        }
        // A ring bursting out of each page just updated, growing and fading.
        for &(index, at) in &self.updates {
            let elapsed = now.saturating_sub(at);
            if elapsed >= UPDATE_RING_MILLISECONDS {
                continue;
            }
            #[allow(clippy::cast_precision_loss, reason = "under two seconds")]
            let progress = elapsed as f64 / UPDATE_RING_MILLISECONDS as f64;
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let centre = Point::new(cells.x0 + ((index % PAGES_PER_BANK) as f64 + 0.5) * width, cells.y0 + ((index / PAGES_PER_BANK) as f64 + 0.5) * height);
            let radius = width.min(height) * (1.0 + UPDATE_RING_SCALE * progress) / 2.0;
            let ring = Rect::new(centre.x - radius, centre.y - radius, centre.x + radius, centre.y + radius);
            border(canvas, ring, UPDATE_RING_WIDTH, mix(UPDATED, PANEL, progress));
        }
        // A dot on each page just received, shrinking away.
        for &(index, start) in &self.pulses {
            let elapsed = now.saturating_sub(start);
            if elapsed >= PULSE_MILLISECONDS {
                continue;
            }
            #[allow(clippy::cast_precision_loss, reason = "under a second")]
            let remaining = 1.0 - elapsed as f64 / PULSE_MILLISECONDS as f64;
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let centre = Point::new(cells.x0 + ((index % PAGES_PER_BANK) as f64 + 0.5) * width, cells.y0 + ((index / PAGES_PER_BANK) as f64 + 0.5) * height);
            let radius = width.min(height) * PULSE_SCALE * remaining / 2.0;
            canvas.fill(Rect::new(centre.x - radius, centre.y - radius, centre.x + radius, centre.y + radius), radius, CELL_PULSE);
        }
    }
}

impl Browser {
    /// The overview's page under the pointer, at its native size, above
    /// its cell. It never covers the tab strip or the trusted strip, where
    /// a page image could pass for the address: on a short window it
    /// shrinks to fit below them.
    fn draw_preview<C: Canvas + ?Sized>(&self, canvas: &mut C, width: f64) {
        let Some(index) = self.previewed else { return };
        let Some(key) = self.page_key(index) else { return };
        let cells = self.regions.cells;
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let (cell_width, cell_height) = (cells.width() / PAGES_PER_BANK as f64, cells.height() / BANKS as f64);
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let (centre, cell_top) = (cells.x0 + ((index % PAGES_PER_BANK) as f64 + 0.5) * cell_width, cells.y0 + (index / PAGES_PER_BANK) as f64 * cell_height);
        let bottom = cell_top - PREVIEW_GAP;
        let scale = ((bottom - BODY_TOP) / f64::from(PAGE_HEIGHT)).min(1.0);
        if scale <= 0.0 {
            return;
        }
        let (page_width, page_height) = (f64::from(PAGE_WIDTH) * scale, f64::from(PAGE_HEIGHT) * scale);
        let left = (centre - page_width / 2.0).clamp(MARGIN, (width - MARGIN - page_width).max(MARGIN));
        let rect = Rect::new(left, bottom - page_height, left + page_width, bottom);
        canvas.dim(Rect::new(rect.x0 + PREVIEW_SHADOW, rect.y0 + PREVIEW_SHADOW, rect.x1 + PREVIEW_SHADOW, rect.y1 + PREVIEW_SHADOW), PREVIEW_SHADOW_DARKNESS);
        canvas.fill(rect, 0.0, BLACK);
        if !canvas.page(key, rect) {
            // Asked for when the pointer arrived; it shows when rendered.
            centred(canvas, rect, &format!("{:03X}", key.page), HEADING, MUTED);
        }
        outline(canvas, rect, WHITE);
    }
}

/// A chevron in `rect`, pointing down or up.
fn chevron<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect, down: bool, colour: Colour) {
    let centre = Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1));
    let (tip, base) = if down { (CHEVRON_HEIGHT / 2.0, -CHEVRON_HEIGHT / 2.0) } else { (-CHEVRON_HEIGHT / 2.0, CHEVRON_HEIGHT / 2.0) };
    let thickness = if down { -CHEVRON_THICKNESS } else { CHEVRON_THICKNESS };
    let half = CHEVRON_WIDTH / 2.0;
    canvas.polygon(
        &[
            Point::new(centre.x - half, centre.y + base),
            Point::new(centre.x, centre.y + tip),
            Point::new(centre.x + half, centre.y + base),
            Point::new(centre.x + half, centre.y + base + thickness),
            Point::new(centre.x, centre.y + tip + thickness),
            Point::new(centre.x - half, centre.y + base + thickness),
        ],
        colour,
    );
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

/// A left- or right-pointing arrow centred in `rect`.
fn arrow<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect, pointing_left: bool, colour: Colour) {
    let centre = Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1));
    let sign = if pointing_left { -1.0 } else { 1.0 };
    let tip = Point::new(centre.x + sign * ARROW_LENGTH / 2.0, centre.y);
    let tail = centre.x - sign * ARROW_LENGTH / 2.0;
    let shaft = Rect::new(
        tail.min(tip.x - sign * ARROW_THICKNESS),
        centre.y - ARROW_THICKNESS / 2.0,
        tail.max(tip.x - sign * ARROW_THICKNESS),
        centre.y + ARROW_THICKNESS / 2.0,
    );
    canvas.fill(shaft, 0.0, colour);
    for direction in [1.0, -1.0] {
        let back = Point::new(tip.x - sign * ARROW_HEAD, tip.y + direction * ARROW_HEAD);
        let thick = Point::new(sign * ARROW_THICKNESS * 0.7, 0.0);
        canvas.polygon(&[tip, back, back + thick, tip + thick], colour);
    }
}

/// A loudspeaker centred in `rect`: with sound waves, or crossed out when
/// muted, as Chrome's tab shows it.
fn speaker_glyph<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect, muted: bool, colour: Colour) {
    let centre = Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1));
    let half = SPEAKER_HALF;
    // The magnet: a square of half the height, left of centre.
    let back = centre.x - half;
    canvas.fill(Rect::new(back, centre.y - half / 2.0, back + half / 2.0, centre.y + half / 2.0), 0.0, colour);
    // The cone opens from the magnet to the full height at the centre.
    let neck = back + half / 2.0;
    canvas.polygon(
        &[
            Point::new(neck, centre.y - half / 2.0),
            Point::new(centre.x, centre.y - half),
            Point::new(centre.x, centre.y + half),
            Point::new(neck, centre.y + half / 2.0),
        ],
        colour,
    );
    let right = centre.x + half / 4.0;
    if muted {
        // A cross to the right of the cone, two bars corner to corner.
        let cross = Point::new(right + half / 2.0, centre.y);
        let reach = half / 2.0;
        let normal = SPEAKER_LINE / 2.0 / core::f64::consts::SQRT_2;
        for direction in [1.0, -1.0] {
            let along = Point::new(reach, reach * direction);
            let across = Point::new(-normal * direction, normal);
            canvas.polygon(&[cross - along + across, cross + along + across, cross + along - across, cross - along - across], colour);
        }
    } else {
        // Two waves, drawn as bars rising with their distance.
        for (offset, height) in [(0.0, half / 2.0), (half / 2.0, half)] {
            let x = right + offset;
            canvas.fill(
                Rect::new(x, centre.y - height / 2.0 - SPEAKER_LINE / 2.0, x + SPEAKER_LINE, centre.y + height / 2.0 + SPEAKER_LINE / 2.0),
                SPEAKER_LINE / 2.0,
                colour,
            );
        }
    }
}

/// A closed padlock with its body's left edge at `at.x`, centred on `at.y`.
fn padlock<C: Canvas + ?Sized>(canvas: &mut C, at: Point, colour: Colour) {
    let body = Rect::new(at.x, at.y - LOCK_HEIGHT / 2.0 + SHACKLE_HEIGHT / 2.0, at.x + LOCK_WIDTH, at.y + LOCK_HEIGHT / 2.0 + SHACKLE_HEIGHT / 2.0);
    canvas.fill(body, 1.5, colour);
    let shackle = Rect::new(
        at.x + (LOCK_WIDTH - SHACKLE_WIDTH) / 2.0,
        body.y0 - SHACKLE_HEIGHT,
        at.x + f64::midpoint(LOCK_WIDTH, SHACKLE_WIDTH),
        body.y0 + SHACKLE_THICKNESS,
    );
    canvas.fill(shackle, SHACKLE_WIDTH / 2.0, colour);
    canvas.fill(
        Rect::new(shackle.x0 + SHACKLE_THICKNESS, shackle.y0 + SHACKLE_THICKNESS, shackle.x1 - SHACKLE_THICKNESS, body.y0),
        (SHACKLE_WIDTH - SHACKLE_THICKNESS * 2.0) / 2.0,
        FIELD,
    );
}

/// A five-pointed star centred on `centre`.
fn star<C: Canvas + ?Sized>(canvas: &mut C, centre: Point, colour: Colour) {
    let mut points = Vec::with_capacity(STAR_POINTS * 2);
    for point in 0..STAR_POINTS * 2 {
        #[allow(clippy::cast_precision_loss, reason = "ten points")]
        let angle = core::f64::consts::PI * point as f64 / STAR_POINTS as f64 - core::f64::consts::FRAC_PI_2;
        let radius = if point % 2 == 0 { STAR_RADIUS } else { STAR_RADIUS * STAR_INNER };
        let (sine, cosine) = libm::sincos(angle);
        points.push(Point::new(centre.x + radius * cosine, centre.y + radius * sine));
    }
    canvas.polygon(&points, colour);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_canvas::Recorder;

    const WIDTH: f64 = 1440.0;
    const HEIGHT: f64 = 900.0;

    fn channel() -> Channel {
        let mut channel = Channel::empty();
        channel.channel = 7;
        channel.title = "Test".into();
        for index in (0..PAGES).filter(|index| index % 3 != 2) {
            channel.pages[index] = Some(PageInfo { title: format!("Page {index}"), status: Status::Available });
        }
        channel
    }

    fn browser() -> (Browser, Recorder) {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: true });
        browser.set_tabs(
            alloc::vec![
                TabInfo { title: "one".into(), colour: 0, channel: 7, muted: false },
                TabInfo { title: "two".into(), colour: 1, channel: 8, muted: false }
            ],
            0,
        );
        browser.set_strip(StripState {
            address: "https://example.invalid/".into(),
            identity: "example.invalid".into(),
            encrypted: true,
            forward: true,
            ..StripState::default()
        });
        browser.set_channel(channel());
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        (browser, canvas)
    }

    fn press_at(browser: &mut Browser, canvas: &mut Recorder, rect: Rect) {
        browser.pointer_moved(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1), 0);
        browser.pointer_pressed(canvas, false, 0);
    }

    #[test]
    fn the_first_bank_is_shown_and_reported_visible() {
        let (mut browser, _) = browser();
        let outputs = browser.take_outputs();
        let Some(Output::Visible(visible)) = outputs.first() else { panic!("{outputs:?}") };
        assert!(visible.iter().all(|key| key.channel == 7 && (0x100..0x200).contains(&key.page)));
        assert_eq!(visible[0].page, 0x100);
    }

    #[test]
    fn choosing_a_bank_shows_its_pages() {
        let (mut browser, mut canvas) = browser();
        browser.take_outputs();
        let bank = browser.regions.banks[4];
        press_at(&mut browser, &mut canvas, bank);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let outputs = browser.take_outputs();
        let Some(Output::Visible(visible)) = outputs.first() else { panic!("{outputs:?}") };
        assert!(visible.iter().all(|key| (0x500..0x600).contains(&key.page)), "{visible:?}");
    }

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
                TabInfo { title: "one".into(), colour: 0, channel: 7, muted: true },
                TabInfo { title: "two".into(), colour: 1, channel: 8, muted: false }
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
                TabInfo { title: "one".into(), colour: 0, channel: 7, muted: true },
                TabInfo { title: "two".into(), colour: 1, channel: 8, muted: false }
            ],
            0,
        );
        press_at(&mut browser, &mut canvas, button);
        browser.set_tabs(
            alloc::vec![
                TabInfo { title: "one".into(), colour: 0, channel: 7, muted: true },
                TabInfo { title: "two".into(), colour: 1, channel: 8, muted: true }
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
        press_at(&mut browser, &mut canvas, forward);
        press_at(&mut browser, &mut canvas, close_window);
        assert_eq!(
            browser.take_outputs(),
            [Output::SelectTab(1), Output::CloseTab(1), Output::Forward, Output::Window(WindowAction::Close)],
            "back is disabled, so it reports nothing"
        );
    }

    #[test]
    fn typing_an_address_navigates() {
        let (mut browser, mut canvas) = browser();
        browser.take_outputs();
        let field = browser.regions.field;
        press_at(&mut browser, &mut canvas, field);
        browser.key(Key::Text("sport.invalid"), Modifiers::default(), 0);
        browser.key(Key::Enter, Modifiers::default(), 0);
        assert_eq!(browser.take_outputs(), [Output::TextInput(true), Output::TextInput(false), Output::Navigate("sport.invalid".into())]);
    }

    #[test]
    fn three_digits_choose_a_page_and_a_received_page_pulses() {
        let (mut browser, _) = browser();
        browser.key(Key::Text("4"), Modifiers::default(), 0);
        browser.key(Key::Text("01"), Modifiers::default(), 0);
        assert_eq!(browser.bank, 3);
        assert_eq!(browser.selected, page_index(0x401));
        // Page 402 is not in the test channel: it arrives now.
        browser.add_page(7, 0x402, "New".into(), 100);
        assert!(browser.animating(500));
        assert!(!browser.animating(100 + PULSE_MILLISECONDS));
        assert_eq!(browser.channel.counts().1, 1);
    }

    #[test]
    fn the_opened_page_is_the_zoomed_one_or_else_the_one_chosen() {
        let (mut browser, mut canvas) = browser();
        assert_eq!(browser.opened(), None);
        browser.key(Key::Text("401"), Modifiers::default(), 0);
        assert_eq!(browser.opened(), Some(PageKey { channel: 7, page: 0x401 }));
        let (key, rect) = browser.wall().shown().next().unwrap();
        press_at(&mut browser, &mut canvas, rect);
        assert_eq!(browser.opened(), Some(key), "a zoomed page wins");
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

    #[test]
    fn a_click_on_the_overview_selects_its_page() {
        let (mut browser, mut canvas) = browser();
        let cells = browser.regions.cells;
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let (width, height) = (cells.width() / PAGES_PER_BANK as f64, cells.height() / BANKS as f64);
        // Page 610: bank 5, column 10.
        let point = Point::new(cells.x0 + 10.5 * width, cells.y0 + 5.5 * height);
        browser.pointer_moved(point.x, point.y, 0);
        browser.pointer_pressed(&mut canvas, false, 0);
        assert_eq!(browser.selected, page_index(0x610));
        assert_eq!(browser.bank, 5);
    }

    /// The overview cell of page `index`'s centre.
    fn cell_centre(browser: &Browser, index: usize) -> Point {
        let cells = browser.regions.cells;
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let (width, height) = (cells.width() / PAGES_PER_BANK as f64, cells.height() / BANKS as f64);
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        Point::new(cells.x0 + ((index % PAGES_PER_BANK) as f64 + 0.5) * width, cells.y0 + ((index / PAGES_PER_BANK) as f64 + 0.5) * height)
    }

    #[test]
    fn a_page_in_the_overview_pops_up_at_native_size_under_a_hand() {
        let (mut browser, mut canvas) = browser();
        browser.add_page(7, 0x610, "Weather".into(), 0);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        browser.take_outputs();
        // Over a page not yet arrived: the arrow, and nothing pops up.
        let waiting = cell_centre(&browser, page_index(0x611).unwrap());
        assert!(!browser.pointer_moved(waiting.x, waiting.y, 0));
        assert_eq!(browser.cursor(), Cursor::Arrow);
        // Over a page that has arrived: a hand, and the page at 480 by 500.
        let point = cell_centre(&browser, page_index(0x610).unwrap());
        assert!(browser.pointer_moved(point.x, point.y, 0));
        assert_eq!(browser.cursor(), Cursor::Hand);
        canvas.pages.clear();
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let key = PageKey { channel: 7, page: 0x610 };
        let &(_, rect) = canvas.pages.iter().rev().find(|(drawn, _)| *drawn == key).expect("the preview is drawn");
        assert_eq!((rect.width(), rect.height()), (f64::from(PAGE_WIDTH), f64::from(PAGE_HEIGHT)));
        assert!(rect.y1 <= point.y && rect.y0 >= BODY_TOP, "above its cell, below the trusted strip: {rect:?}");
        // It is reported visible, so the trusted core has it rendered.
        let outputs = browser.take_outputs();
        assert!(outputs.iter().any(|output| matches!(output, Output::Visible(visible) if visible.contains(&key))), "{outputs:?}");
        // On a short window it shrinks rather than cover the strip.
        let short = BODY_TOP + OVERVIEW_HEIGHT + 300.0;
        browser.draw(&mut canvas, WIDTH, short, 0);
        let point = cell_centre(&browser, page_index(0x610).unwrap());
        browser.pointer_moved(point.x, point.y, 0);
        canvas.pages.clear();
        browser.draw(&mut canvas, WIDTH, short, 0);
        let &(_, rect) = canvas.pages.iter().rev().find(|(drawn, _)| *drawn == key).expect("the preview is drawn");
        assert!(rect.y0 >= BODY_TOP && rect.height() < f64::from(PAGE_HEIGHT), "{rect:?}");
    }

    #[test]
    fn the_overview_folds_into_the_footer_and_rests_there() {
        let (mut browser, mut canvas) = browser();
        let open_cells = browser.regions.cells;
        let wall_before = browser.regions.list;
        let toggle = browser.regions.overview_toggle;
        press_at(&mut browser, &mut canvas, toggle);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        // Folded: one heading high at the bottom, no cells, the panels above taller.
        assert!((browser.regions.overview.height() - HEADING_HEIGHT).abs() < 1e-9);
        assert_eq!(browser.regions.cells, Rect::ZERO);
        assert!(browser.regions.list.height() > wall_before.height());
        // A page arriving starts no animation and asks for no frames.
        browser.add_page(7, 0x105, "Arrived while folded".into(), 1_000);
        assert!(!browser.animating(1_000));
        assert!(!browser.pointer_moved(open_cells.x0 + 1.0, open_cells.y0 + 1.0, 1_000), "nothing to hover");
        assert_eq!(browser.cursor(), Cursor::Arrow);
        // Opened again: drawn as things are now, the page that arrived loaded.
        let toggle = browser.regions.overview_toggle;
        press_at(&mut browser, &mut canvas, toggle);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 2_000);
        assert_eq!(browser.regions.cells, open_cells);
        let point = cell_centre(&browser, page_index(0x105).unwrap());
        browser.pointer_moved(point.x, point.y, 2_000);
        assert_eq!(browser.cursor(), Cursor::Hand);
    }

    #[test]
    fn the_pointer_is_a_hand_over_the_overview_fold_button() {
        let (mut browser, mut canvas) = browser();
        for folded in [false, true] {
            let toggle = browser.regions.overview_toggle;
            browser.pointer_moved(f64::midpoint(toggle.x0, toggle.x1), f64::midpoint(toggle.y0, toggle.y1), 0);
            assert_eq!(browser.cursor(), Cursor::Hand, "folded: {folded}");
            browser.pointer_moved(toggle.x0 - 1.0, f64::midpoint(toggle.y0, toggle.y1), 0);
            assert_eq!(browser.cursor(), Cursor::Arrow, "folded: {folded}");
            press_at(&mut browser, &mut canvas, toggle);
            browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        }
    }
}
