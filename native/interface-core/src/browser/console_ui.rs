//! The developer console (`settings-home-console-proxy.md` section 3,
//! requirement 19.8): a panel docked at the bottom of the window, in place
//! of the overview, opened by Command-Option-I on macOS and Control-Shift-I
//! elsewhere, with a tab each for the active channel's network exchanges,
//! packets, pages and notices, and one for managing cookies
//! (`cookies_ui.rs`). What it shows is the trusted core's cleaned text,
//! drawn as text and never interpreted. It can pause, which stops it
//! scrolling, and clear.

use alloc::collections::VecDeque;
use alloc::format;
use alloc::vec::Vec;

use super::widgets::field;
use super::{Browser, Key, Output};
use crate::canvas::{Canvas, Style};
use crate::console::{ConsoleEntry, ConsoleTab, ConsoleUpdate, KEPT, Level};
use crate::editor::Editor;
use crate::geometry::{Point, Rect};
use crate::theme::{BUTTON, DIM, HEADING, MUTED, PANEL, RED, ROW_STATUS, ROW_TITLE, RULE, SELECTED, WHITE, YELLOW};
use crate::ui::{border, centred, left_centred, outline};

/// The Log tab's level buttons and its search, as a web browser's console
/// has them (decision 70).
const LEVEL_WIDTH: f64 = 86.0;
const SEARCH_WIDTH: f64 = 220.0;
/// The longest search: a log line's length.
const MAX_SEARCH: usize = crate::console::MAX_ENTRY;

/// The console's height, in place of the overview.
pub(super) const CONSOLE_HEIGHT: f64 = 260.0;
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
    pub(super) area: Rect,
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
    pub(super) const fn open(&self) -> bool {
        self.open
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
                // At most a tab's worth of each tab waits.
                if console.held.len() < KEPT * ConsoleTab::ALL.len() {
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
    pub(super) fn toggle_console(&mut self) {
        self.console.open = !self.console.open;
        self.outputs.push(Output::Console(self.console.open));
        if self.console.tab == Tab::Cookies {
            self.set_cookies_open(self.console.open);
        }
    }

    /// The console, open at its cookies' tab, from the settings panel.
    pub(super) fn manage_cookies(&mut self) {
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

    /// A press on the console; returns whether it was the console's.
    pub(super) fn console_press(&mut self, point: Point) -> bool {
        if !self.console.open || !self.console.area.contains(point) {
            return false;
        }
        // The log's levels and search (decision 70).
        if let Some(&(level, _)) = self.console.levels.iter().find(|(_, rect)| rect.contains(point)) {
            if let Some(hidden) = self.console.hidden.get_mut(level_index(level)) {
                *hidden = !*hidden;
            }
            self.console.back = 0;
            return true;
        }
        if self.console.search_field.contains(point) {
            if !self.console.searching {
                self.console.searching = true;
                self.outputs.push(Output::TextInput(true));
            }
            return true;
        }
        self.stop_console_search();
        if let Some(&(tab, _)) = self.console.tabs.iter().find(|(_, rect)| rect.contains(point)) {
            self.choose_console_tab(tab);
        } else if self.console.close.contains(point) {
            self.toggle_console();
        } else if self.console.tab == Tab::Cookies {
            self.cookies_press(point);
        } else if self.console.pause.contains(point) {
            self.set_paused(!self.console.paused);
        } else if self.console.clear.contains(point) {
            self.console.entries.iter_mut().for_each(VecDeque::clear);
            self.console.held.clear();
            self.console.back = 0;
        }
        true
    }

    fn stop_console_search(&mut self) {
        if self.console.searching {
            self.console.searching = false;
            self.outputs.push(Output::TextInput(false));
        }
    }

    /// Whether the log's search takes the keys.
    pub(super) fn console_searching(&self) -> bool {
        self.console.open && self.console.searching
    }

    /// The log's search, while it has the keys.
    pub(super) fn console_field(&mut self) -> Option<&mut Editor> {
        self.console_searching().then_some(&mut self.console.search)
    }

    /// A key while the log's search takes them; `None` when it does not.
    pub(super) fn console_key(&mut self, key: Key<'_>, shift: bool) -> Option<bool> {
        if !self.console_searching() {
            return None;
        }
        let search = &mut self.console.search;
        match key {
            Key::Text(text) => search.insert(text),
            Key::Backspace => search.backspace(),
            Key::Delete => search.delete(),
            Key::Left => search.left(shift),
            Key::Right => search.right(shift),
            Key::Home | Key::Up => search.home(shift),
            Key::End | Key::Down => search.end(shift),
            Key::Escape | Key::Tab | Key::Enter => self.stop_console_search(),
        }
        self.console.back = 0;
        Some(true)
    }

    /// The wheel over the console scrolls its entries; returns whether it
    /// was over it.
    pub(super) fn console_scroll(&mut self, distance: f64) -> bool {
        if !self.console.open || !self.console.area.contains(self.pointer) {
            return false;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a few lines of wheel")]
        let lines = libm::ceil(distance.abs() / LINE) as usize;
        let most = self.console.shown().map_or(0, |entries| entries.len());
        self.console.back = if distance < 0.0 { self.console.back.saturating_add(lines).min(most) } else { self.console.back.saturating_sub(lines) };
        true
    }

    /// The console's tabs and buttons, and the cookies' controls.
    pub(super) fn console_tips(&self, tips: &mut Vec<(Rect, alloc::string::String)>) {
        if !self.console.open {
            return;
        }
        for &(tab, rect) in &self.console.tabs {
            let words = match tab {
                Tab::Entries(ConsoleTab::Network) => "Each exchange with the channel's server, and the push connection's events",
                Tab::Entries(ConsoleTab::Packets) => "Each teletext packet received",
                Tab::Entries(ConsoleTab::Pages) => "The pages the decoder holds",
                Tab::Entries(ConsoleTab::Log) => "The browser's log: each process's errors, warnings and notices, and how it was confined",
                Tab::Cookies => "Cookies kept for each site and channel: delete or block them",
            };
            tips.push((rect, words.into()));
        }
        let close = format!("Close the console ({})", self.keys_named("Option+Cmd+I", "Ctrl+Shift+I"));
        tips.push((self.console.close, close));
        if self.console.tab == Tab::Cookies {
            self.cookies_tips(tips);
        } else {
            tips.push((
                self.console.pause,
                if self.console.paused { "Scroll again, with what arrived meanwhile" } else { "Stop scrolling; what arrives waits" }.into(),
            ));
            tips.push((self.console.clear, "Empty every tab's entries".into()));
        }
    }

    /// The console, docked at the bottom in `area`.
    pub(super) fn draw_console<C: Canvas + ?Sized>(&mut self, canvas: &mut C, area: Rect) {
        self.console.area = area;
        self.console.tabs.clear();
        canvas.fill(area, 0.0, PANEL);
        outline(canvas, area, RULE);
        let bar = Rect::new(area.x0 + PADDING, area.y0 + GAP, area.x1 - PADDING, area.y0 + GAP + BAR_HEIGHT);
        let mut x = bar.x0;
        for tab in TABS {
            let rect = Rect::new(x, bar.y0, x + TAB_WIDTH, bar.y1);
            let chosen = self.console.tab == tab;
            canvas.fill(rect, RADIUS, if chosen { SELECTED } else { BUTTON });
            border(canvas, rect, 1.0, if chosen { SELECTED } else { RULE });
            let label = match tab {
                Tab::Entries(tab) => format!("{} {}", tab.label(), self.console.entries[tab.index()].len()),
                Tab::Cookies => "Cookies".into(),
            };
            centred(canvas, rect, &label, ROW_STATUS, WHITE);
            self.console.tabs.push((tab, rect));
            x = rect.x1 + GAP;
        }
        self.console.close = Rect::new(bar.x1 - CLOSE_WIDTH, bar.y0, bar.x1, bar.y1);
        // Pause and Clear are the entries'; the cookies' tab has its own controls.
        let entries_shown = self.console.tab != Tab::Cookies;
        let (clear, pause) = if entries_shown {
            let clear = Rect::new(self.console.close.x0 - GAP - BUTTON_WIDTH, bar.y0, self.console.close.x0 - GAP, bar.y1);
            (clear, Rect::new(clear.x0 - GAP - BUTTON_WIDTH, bar.y0, clear.x0 - GAP, bar.y1))
        } else {
            (Rect::default(), Rect::default())
        };
        (self.console.clear, self.console.pause) = (clear, pause);
        let mut buttons = alloc::vec![(self.console.close, "\u{00D7}", false)];
        if entries_shown {
            buttons.push((self.console.pause, if self.console.paused { "Resume" } else { "Pause" }, self.console.paused));
            buttons.push((self.console.clear, "Clear", false));
        }
        for (rect, word, chosen) in buttons {
            canvas.fill(rect, RADIUS, if chosen { SELECTED } else { BUTTON });
            border(canvas, rect, 1.0, RULE);
            centred(canvas, rect, word, ROW_TITLE, WHITE);
        }
        let leftmost = if entries_shown { self.console.pause.x0 } else { self.console.close.x0 };
        let title = Rect::new(x + GAP, bar.y0, leftmost - GAP, bar.y1);
        self.console.levels.clear();
        self.console.search_field = Rect::ZERO;
        if self.console.tab == Tab::Entries(ConsoleTab::Log) {
            // The log's levels, each shown or not, and its search.
            let mut left = title.x0;
            for level in Level::ALL {
                let rect = Rect::new(left, bar.y0, left + LEVEL_WIDTH, bar.y1);
                if rect.x1 > title.x1 {
                    break;
                }
                let shown = !self.console.hidden.get(level_index(level)).copied().unwrap_or(false);
                canvas.fill(rect, RADIUS, if shown { SELECTED } else { BUTTON });
                border(canvas, rect, 1.0, RULE);
                centred(canvas, rect, level.label(), ROW_STATUS, if shown { WHITE } else { DIM });
                self.console.levels.push((level, rect));
                left = rect.x1 + GAP;
            }
            let search = Rect::new(left, bar.y0, (left + SEARCH_WIDTH).min(title.x1), bar.y1);
            if search.width() > GAP * 4.0 {
                field(canvas, search, &self.console.search, self.console.searching);
                if self.console.search.text().is_empty() && !self.console.searching {
                    left_centred(canvas, search, GAP, "Search the log", ROW_STATUS, DIM);
                }
                self.console.search_field = search;
            }
        } else {
            canvas.push_clip(title);
            left_centred(canvas, title, 0.0, "Console", HEADING, MUTED);
            canvas.pop_clip();
        }

        let list = Rect::new(area.x0 + PADDING, bar.y1 + GAP, area.x1 - PADDING, area.y1 - GAP);
        let Some(entries) = self.console.shown() else {
            self.draw_cookies(canvas, list);
            return;
        };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a panel's lines")]
        let rows = libm::floor(list.height() / LINE).max(0.0) as usize;
        let end = entries.len().saturating_sub(self.console.back);
        let start = end.saturating_sub(rows);
        canvas.push_clip(list);
        if entries.is_empty() {
            left_centred(canvas, Rect::new(list.x0, list.y0, list.x1, list.y0 + LINE), 0.0, "Nothing yet", ENTRY, DIM);
        }
        for (row, entry) in entries.get(start..end).unwrap_or_default().iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "a panel's lines")]
            let top = list.y0 + row as f64 * LINE;
            let line = Rect::new(list.x0, top, list.x1, top + LINE);
            let time = format!("{}.{:03} s", entry.at_ms / MILLISECONDS_PER_SECOND, entry.at_ms % MILLISECONDS_PER_SECOND);
            left_centred(canvas, line, 0.0, &time, ENTRY, MUTED);
            left_centred(canvas, line, TIME_WIDTH, &entry.text, ENTRY, level_colour(entry.level));
        }
        canvas.pop_clip();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{Frame, Key, Modifiers};
    use crate::test_canvas::Recorder;

    fn entry(tab: ConsoleTab, text: &str) -> ConsoleEntry {
        ConsoleEntry { tab, level: Level::Info, at_ms: 1_250, text: text.into() }
    }

    fn log(level: Level, text: &str) -> ConsoleEntry {
        ConsoleEntry { tab: ConsoleTab::Log, level, at_ms: 2_000, text: text.into() }
    }

    /// Decision 70: the Log tab shows each line in its level's colour, and
    /// its level buttons and search choose which lines show.
    #[test]
    fn the_log_filters_by_level_and_search() {
        let mut browser = open();
        browser.console_update(ConsoleUpdate {
            channel: 1,
            replace: true,
            entries: alloc::vec![
                log(Level::Error, "decoder: stopped: broken pipe"),
                log(Level::Warning, "fetcher: 404 for 205.tti"),
                log(Level::Info, "fetcher: confined: sandbox-exec"),
            ],
        });
        texts(&mut browser);
        let tab = browser.console.tabs.iter().find(|(tab, _)| *tab == Tab::Entries(ConsoleTab::Log)).unwrap().1;
        press(&mut browser, tab);
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, 1440.0, 900.0, 0);
        let colour_of = |canvas: &Recorder, text: &str| canvas.coloured_text(text);
        assert_eq!(colour_of(&canvas, "decoder: stopped: broken pipe"), Some(RED));
        assert_eq!(colour_of(&canvas, "fetcher: 404 for 205.tti"), Some(YELLOW));
        let info = browser.console.levels.iter().find(|(level, _)| *level == Level::Info).unwrap().1;
        press(&mut browser, info);
        let shown = texts(&mut browser);
        assert!(!shown.contains(&"fetcher: confined: sandbox-exec".into()) && shown.contains(&"fetcher: 404 for 205.tti".into()), "information hidden");
        let search = browser.console.search_field;
        press(&mut browser, search);
        assert_eq!(browser.take_outputs(), [Output::TextInput(true)]);
        browser.key(Key::Text("DECODER"), Modifiers::default(), 0);
        let shown = texts(&mut browser);
        assert!(shown.contains(&"decoder: stopped: broken pipe".into()) && !shown.contains(&"fetcher: 404 for 205.tti".into()), "searched, in any case");
        browser.key(Key::Escape, Modifiers::default(), 0);
        assert_eq!(browser.take_outputs(), [Output::TextInput(false)]);
    }

    fn texts(browser: &mut Browser) -> Vec<alloc::string::String> {
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, 1440.0, 900.0, 0);
        canvas.texts.into_iter().map(|(_, text)| text).collect()
    }

    fn press(browser: &mut Browser, rect: Rect) {
        browser.pointer_moved(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1), 0);
        browser.pointer_pressed(&mut Recorder::new(), false, 0);
    }

    fn open() -> Browser {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
        browser.key(Key::Text("i"), Modifiers { command: true, alt: true, ..Modifiers::default() }, 0);
        assert_eq!(browser.take_outputs(), [Output::Console(true)]);
        browser
    }

    /// Requirement 19.8: each tab shows its entries, as text.
    #[test]
    fn each_tab_shows_its_entries() {
        let mut browser = open();
        let hostile = "<script>alert(1)</script> \u{1b}[31m";
        browser.console_update(ConsoleUpdate {
            channel: 1,
            replace: true,
            entries: alloc::vec![
                entry(ConsoleTab::Network, "GET https://a.example/100.tti 200 512 bytes 12 ms"),
                entry(ConsoleTab::Packets, "pushed M1 R05  Hello  48 65"),
                entry(ConsoleTab::Pages, "page 100 arrived: News"),
                entry(ConsoleTab::Log, hostile),
            ],
        });
        let shown = texts(&mut browser);
        assert!(shown.iter().any(|text| text.starts_with("GET https://a.example/")), "{shown:?}");
        assert!(shown.contains(&"1.250 s".into()));
        assert!(!shown.iter().any(|text| text.starts_with("pushed")), "another tab's entries are not shown");
        for (index, expected) in [(1, "pushed M1 R05  Hello  48 65"), (2, "page 100 arrived: News"), (3, hostile)] {
            let tab = browser.console.tabs[index].1;
            press(&mut browser, tab);
            assert!(texts(&mut browser).contains(&expected.into()), "{expected}: shown as the text it is");
        }
    }

    /// Requirement 19.8: pause stops it scrolling, clear empties it; a
    /// channel's entries replace another's.
    #[test]
    fn pause_holds_and_clear_empties() {
        let mut browser = open();
        texts(&mut browser);
        let update = |text: &str| ConsoleUpdate { channel: 1, replace: false, entries: alloc::vec![entry(ConsoleTab::Network, text)] };
        browser.console_update(update("first"));
        let pause = browser.console.pause;
        press(&mut browser, pause);
        browser.console_update(update("second"));
        let shown = texts(&mut browser);
        assert!(shown.contains(&"first".into()) && !shown.contains(&"second".into()) && shown.contains(&"Resume".into()));
        press(&mut browser, pause);
        assert!(texts(&mut browser).contains(&"second".into()), "resumed");
        let clear = browser.console.clear;
        press(&mut browser, clear);
        assert!(texts(&mut browser).contains(&"Nothing yet".into()));
        browser.console_update(update("third"));
        browser.console_update(ConsoleUpdate { channel: 2, replace: true, entries: alloc::vec![entry(ConsoleTab::Network, "other")] });
        let shown = texts(&mut browser);
        assert!(shown.contains(&"other".into()) && !shown.contains(&"third".into()));
        for n in 0..KEPT + 5 {
            browser.console_update(ConsoleUpdate { channel: 2, replace: false, entries: alloc::vec![entry(ConsoleTab::Pages, &format!("{n}"))] });
        }
        assert_eq!(browser.console.entries[ConsoleTab::Pages.index()].len(), KEPT);
        let close = browser.console.close;
        press(&mut browser, close);
        assert_eq!(browser.take_outputs(), [Output::Console(false)]);
        assert!(!browser.console_open());
    }

    /// Requirement 19.8 (decision 63): choosing the Cookies tab asks for
    /// the cookies' rows; choosing another stops them.
    #[test]
    fn the_cookies_tab_asks_for_rows_and_another_stops_them() {
        let mut browser = open();
        texts(&mut browser);
        let cookies = browser.console.tabs.iter().find(|(tab, _)| *tab == Tab::Cookies).unwrap().1;
        press(&mut browser, cookies);
        assert_eq!(browser.take_outputs(), [Output::Cookies(crate::cookies::CookiesRequest::Panel(true))]);
        assert!(browser.cookies_open());
        texts(&mut browser);
        let network = browser.console.tabs[0].1;
        press(&mut browser, network);
        assert_eq!(browser.take_outputs(), [Output::Cookies(crate::cookies::CookiesRequest::Panel(false))]);
        assert!(!browser.cookies_open());
    }

    #[test]
    fn its_shortcut_on_each_platform() {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: true });
        browser.key(Key::Text("I"), Modifiers { command: true, shift: true, ..Modifiers::default() }, 0);
        assert_eq!(browser.take_outputs(), [Output::Console(true)], "Control-Shift-I");
        browser.key(Key::Text("i"), Modifiers { command: true, alt: true, ..Modifiers::default() }, 0);
        assert_eq!(browser.take_outputs(), [Output::Console(false)], "Command-Option-I");
    }
}
