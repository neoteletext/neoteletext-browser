//! The developer console (`settings-home-console-proxy.md` section 3,
//! requirement 19.8): a panel docked at the bottom of the window, in place
//! of the overview, opened by Command-Option-I on macOS and Control-Shift-I
//! elsewhere, with a tab each for the active channel's network exchanges,
//! packets, pages and notices. What it shows is the trusted core's cleaned
//! text, drawn as text and never interpreted. It can pause, which stops it
//! scrolling, and clear.

use alloc::collections::VecDeque;
use alloc::format;
use alloc::vec::Vec;

use super::{Browser, Output};
use crate::canvas::{Canvas, Style};
use crate::console::{ConsoleEntry, ConsoleTab, ConsoleUpdate, KEPT};
use crate::geometry::{Point, Rect};
use crate::theme::{BUTTON, DIM, HEADING, MUTED, PANEL, ROW_STATUS, ROW_TITLE, RULE, SELECTED, WHITE};
use crate::ui::{border, centred, left_centred, outline};

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
#[derive(Debug, Default)]
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
    tabs: Vec<(ConsoleTab, Rect)>,
    pause: Rect,
    clear: Rect,
    close: Rect,
}

/// The tab shown: [`ConsoleTab`] with a default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Tab(ConsoleTab);

impl Default for Tab {
    fn default() -> Self {
        Self(ConsoleTab::Network)
    }
}

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

    fn shown(&self) -> &VecDeque<ConsoleEntry> {
        let index = self.tab.0.index();
        &self.entries[index]
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
        if let Some(&(tab, _)) = self.console.tabs.iter().find(|(_, rect)| rect.contains(point)) {
            self.console.tab = Tab(tab);
            self.console.back = 0;
        } else if self.console.pause.contains(point) {
            self.set_paused(!self.console.paused);
        } else if self.console.clear.contains(point) {
            self.console.entries.iter_mut().for_each(VecDeque::clear);
            self.console.held.clear();
            self.console.back = 0;
        } else if self.console.close.contains(point) {
            self.toggle_console();
        }
        true
    }

    /// The wheel over the console scrolls its entries; returns whether it
    /// was over it.
    pub(super) fn console_scroll(&mut self, distance: f64) -> bool {
        if !self.console.open || !self.console.area.contains(self.pointer) {
            return false;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a few lines of wheel")]
        let lines = libm::ceil(distance.abs() / LINE) as usize;
        let most = self.console.shown().len();
        self.console.back = if distance < 0.0 { self.console.back.saturating_add(lines).min(most) } else { self.console.back.saturating_sub(lines) };
        true
    }

    /// The console, docked at the bottom in `area`.
    pub(super) fn draw_console<C: Canvas + ?Sized>(&mut self, canvas: &mut C, area: Rect) {
        self.console.area = area;
        self.console.tabs.clear();
        canvas.fill(area, 0.0, PANEL);
        outline(canvas, area, RULE);
        let bar = Rect::new(area.x0 + PADDING, area.y0 + GAP, area.x1 - PADDING, area.y0 + GAP + BAR_HEIGHT);
        let mut x = bar.x0;
        for tab in ConsoleTab::ALL {
            let rect = Rect::new(x, bar.y0, x + TAB_WIDTH, bar.y1);
            let chosen = self.console.tab.0 == tab;
            canvas.fill(rect, RADIUS, if chosen { SELECTED } else { BUTTON });
            border(canvas, rect, 1.0, if chosen { SELECTED } else { RULE });
            let count = self.console.entries[tab.index()].len();
            centred(canvas, rect, &format!("{} {count}", tab.label()), ROW_STATUS, WHITE);
            self.console.tabs.push((tab, rect));
            x = rect.x1 + GAP;
        }
        self.console.close = Rect::new(bar.x1 - CLOSE_WIDTH, bar.y0, bar.x1, bar.y1);
        self.console.clear = Rect::new(self.console.close.x0 - GAP - BUTTON_WIDTH, bar.y0, self.console.close.x0 - GAP, bar.y1);
        self.console.pause = Rect::new(self.console.clear.x0 - GAP - BUTTON_WIDTH, bar.y0, self.console.clear.x0 - GAP, bar.y1);
        for (rect, word, chosen) in [
            (self.console.pause, if self.console.paused { "Resume" } else { "Pause" }, self.console.paused),
            (self.console.clear, "Clear", false),
            (self.console.close, "\u{00D7}", false),
        ] {
            canvas.fill(rect, RADIUS, if chosen { SELECTED } else { BUTTON });
            border(canvas, rect, 1.0, RULE);
            centred(canvas, rect, word, ROW_TITLE, WHITE);
        }
        let title = Rect::new(x + GAP, bar.y0, self.console.pause.x0 - GAP, bar.y1);
        canvas.push_clip(title);
        left_centred(canvas, title, 0.0, "Console", HEADING, MUTED);
        canvas.pop_clip();

        let list = Rect::new(area.x0 + PADDING, bar.y1 + GAP, area.x1 - PADDING, area.y1 - GAP);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a panel's lines")]
        let rows = libm::floor(list.height() / LINE).max(0.0) as usize;
        let entries = self.console.shown();
        let end = entries.len().saturating_sub(self.console.back);
        let start = end.saturating_sub(rows);
        canvas.push_clip(list);
        if entries.is_empty() {
            left_centred(canvas, Rect::new(list.x0, list.y0, list.x1, list.y0 + LINE), 0.0, "Nothing yet", ENTRY, DIM);
        }
        for (row, entry) in entries.range(start..end).enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "a panel's lines")]
            let top = list.y0 + row as f64 * LINE;
            let line = Rect::new(list.x0, top, list.x1, top + LINE);
            let time = format!("{}.{:03} s", entry.at_ms / MILLISECONDS_PER_SECOND, entry.at_ms % MILLISECONDS_PER_SECOND);
            left_centred(canvas, line, 0.0, &time, ENTRY, MUTED);
            left_centred(canvas, line, TIME_WIDTH, &entry.text, ENTRY, WHITE);
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
        ConsoleEntry { tab, at_ms: 1_250, text: text.into() }
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
                entry(ConsoleTab::Notices, hostile),
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

    #[test]
    fn its_shortcut_on_each_platform() {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: true });
        browser.key(Key::Text("I"), Modifiers { command: true, shift: true, ..Modifiers::default() }, 0);
        assert_eq!(browser.take_outputs(), [Output::Console(true)], "Control-Shift-I");
        browser.key(Key::Text("i"), Modifiers { command: true, alt: true, ..Modifiers::default() }, 0);
        assert_eq!(browser.take_outputs(), [Output::Console(false)], "Command-Option-I");
    }
}
