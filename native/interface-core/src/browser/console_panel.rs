//! The developer console (`settings-home-console-proxy.md` section 3,
//! requirement 19.8): a panel docked at the bottom of the window, in place
//! of the overview, opened by Command-Option-I on macOS and Control-Shift-I
//! elsewhere, with a tab each for the active channel's network exchanges,
//! packets, pages and notices, and one for managing cookies
//! (`cookies_tab.rs`). What it shows is the trusted core's cleaned text,
//! drawn as text and never interpreted. It can pause, which stops it
//! scrolling, and clear.

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use crate::browser::{Browser, Output};
use crate::canvas::Style;
use crate::console::{ConsoleEntry, ConsoleTab, ConsoleUpdate, KEPT, Level};
use crate::editor::Editor;
use crate::geometry::Rect;
use crate::theme::{RED, WHITE, YELLOW};

mod draw;
mod input;

/// The Log tab's level buttons and its search, as a web browser's console
/// has them (decision 70).
const LEVEL_WIDTH: f64 = 86.0;
const SEARCH_WIDTH: f64 = 220.0;
/// The longest search: a log line's length.
const MAX_SEARCH: usize = crate::console::MAX_ENTRY;
/// The entries held while the console is paused: at most a tab's worth of
/// each tab.
const HELD: usize = KEPT * ConsoleTab::ALL.len();

/// The console's height, in place of the overview.
pub(in crate::browser) const CONSOLE_HEIGHT: f64 = 260.0;
const BAR_HEIGHT: f64 = 34.0;
const TAB_WIDTH: f64 = 120.0;
const BUTTON_WIDTH: f64 = 84.0;
const CLOSE_WIDTH: f64 = 34.0;
const GAP: f64 = 6.0;
const PADDING: f64 = 10.0;
const LINE: f64 = 17.0;
const TIME_WIDTH: f64 = 92.0;
const RADIUS: f64 = 4.0;
const MILLISECONDS_PER_SECOND: u64 = 1000;
/// Entries in a fixed-width face, so packets' bytes line up.
const ENTRY: Style = Style { size: 12.0, monospace: true, bold: false, caps: false };

/// The console's state in the browser.
pub(super) struct ConsoleUi {
    open: bool,
    /// The channel the entries are of.
    channel: Option<u32>,
    entries: [VecDeque<ConsoleEntry>; ConsoleTab::ALL.len()],
    /// While paused, what arrives waits here, so nothing scrolls.
    held: Vec<ConsoleEntry>,
    paused: bool,
    tab: Tab,
    /// Lines scrolled back from the newest.
    back: usize,
    area: Rect,
    tabs: Vec<(Tab, Rect)>,
    pause: Rect,
    clear: Rect,
    close: Rect,
    /// The Log tab's levels not shown, in [`Level::ALL`]'s order.
    hidden: [bool; 3],
    /// Its search, and whether it takes the keys.
    search: Editor,
    searching: bool,
    levels: Vec<(Level, Rect)>,
    search_field: Rect,
}

impl Default for ConsoleUi {
    fn default() -> Self {
        Self {
            open: false,
            channel: None,
            entries: Default::default(),
            held: Vec::new(),
            paused: false,
            tab: Tab::default(),
            back: 0,
            area: Rect::ZERO,
            tabs: Vec::new(),
            pause: Rect::ZERO,
            clear: Rect::ZERO,
            close: Rect::ZERO,
            hidden: [false; 3],
            search: Editor::new(MAX_SEARCH),
            searching: false,
            levels: Vec::new(),
            search_field: Rect::ZERO,
        }
    }
}

/// A level's place in [`Level::ALL`].
fn level_index(level: Level) -> usize {
    Level::ALL.iter().position(|each| *each == level).unwrap_or(0)
}

/// A log line's colour: errors red, warnings yellow, as a web browser's.
const fn level_colour(level: Level) -> crate::theme::Colour {
    match level {
        Level::Error => RED,
        Level::Warning => YELLOW,
        Level::Info => WHITE,
    }
}

/// The tab shown: one of the trusted core's entries, or the cookies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tab {
    Entries(ConsoleTab),
    Cookies,
}

impl Default for Tab {
    fn default() -> Self {
        Self::Entries(ConsoleTab::Network)
    }
}

/// Every tab, in the order the console shows them.
const TABS: [Tab; ConsoleTab::ALL.len() + 1] =
    [Tab::Entries(ConsoleTab::ALL[0]), Tab::Entries(ConsoleTab::ALL[1]), Tab::Entries(ConsoleTab::ALL[2]), Tab::Entries(ConsoleTab::ALL[3]), Tab::Cookies];

impl ConsoleUi {
    pub(in crate::browser) const fn open(&self) -> bool {
        self.open
    }

    /// Where the console was last drawn.
    #[cfg(test)]
    pub(in crate::browser) const fn area(&self) -> Rect {
        self.area
    }

    fn keep(&mut self, entry: ConsoleEntry) {
        let Some(entries) = self.entries.get_mut(entry.tab.index()) else { return };
        if entries.len() >= KEPT {
            entries.pop_front();
        }
        entries.push_back(entry);
    }

    /// The entries of the tab shown, those the Log tab's levels and search
    /// let through; none on the cookies' tab.
    fn shown(&self) -> Option<Vec<&ConsoleEntry>> {
        let Tab::Entries(tab) = self.tab else { return None };
        let entries = self.entries.get(tab.index())?;
        if tab != ConsoleTab::Log {
            return Some(entries.iter().collect());
        }
        let search = self.search.text().to_lowercase();
        Some(
            entries
                .iter()
                .filter(|entry| !self.hidden.get(level_index(entry.level)).copied().unwrap_or(false))
                .filter(|entry| search.is_empty() || entry.text.to_lowercase().contains(&search))
                .collect(),
        )
    }
}

impl Browser {
    /// Entries the trusted core sent for the console.
    pub fn console_update(&mut self, update: ConsoleUpdate) {
        let console = &mut self.console;
        if update.replace || console.channel != Some(update.channel) {
            console.channel = Some(update.channel);
            console.entries.iter_mut().for_each(VecDeque::clear);
            console.held.clear();
            console.back = 0;
        }
        for entry in update.entries {
            if console.paused {
                if console.held.len() < HELD {
                    console.held.push(entry);
                }
            } else {
                console.keep(entry);
            }
        }
    }

    /// Whether the console is open.
    #[must_use]
    pub fn console_open(&self) -> bool {
        self.console.open
    }

    /// Its shortcut: the console opens, or closes, and the trusted core
    /// starts or stops sending it.
    pub(in crate::browser) fn toggle_console(&mut self) {
        self.console.open = !self.console.open;
        self.outputs.push(Output::Console(self.console.open));
        if self.console.tab == Tab::Cookies {
            self.set_cookies_open(self.console.open);
        }
    }

    /// The console, open at its cookies' tab, from the settings panel.
    pub(in crate::browser) fn manage_cookies(&mut self) {
        self.close_panels();
        if !self.console.open {
            self.toggle_console();
        }
        self.choose_console_tab(Tab::Cookies);
    }

    fn choose_console_tab(&mut self, tab: Tab) {
        self.console.tab = tab;
        self.console.back = 0;
        self.set_cookies_open(tab == Tab::Cookies);
    }

    fn set_paused(&mut self, paused: bool) {
        self.console.paused = paused;
        if !paused {
            for entry in core::mem::take(&mut self.console.held) {
                self.console.keep(entry);
            }
            self.console.back = 0;
        }
    }
}

#[cfg(test)]
mod tests;
