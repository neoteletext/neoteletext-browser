//! The console drawn: its tabs and buttons, the Log tab's levels and
//! search, the cookies' tab or the entries, none of them while it is
//! minimised to its tab bar; and its tips.

use alloc::collections::VecDeque;
use alloc::format;
use alloc::vec::Vec;

use super::{
    BAR_HEIGHT, BUTTON_WIDTH, ENTRY, GAP, LEVEL_WIDTH, LINE, MILLISECONDS_PER_SECOND, PADDING, RADIUS, SEARCH_WIDTH, TAB_WIDTH, TABS, TIME_WIDTH, Tab,
    WINDOW_BUTTON_WIDTH, level_colour, level_index,
};
use crate::browser::Browser;
use crate::browser::glyphs::{chevron, close_glyph, maximise_glyph};
use crate::browser::widgets::field;
use crate::canvas::Canvas;
use crate::console::{ConsoleTab, Level};
use crate::geometry::Rect;
use crate::text::{border, centred, left_centred, outline};
use crate::theme::{BUTTON, DIM, HEADING, MUTED, PANEL, ROW_STATUS, ROW_TITLE, RULE, SELECTED, WHITE};

/// The Stats tab's column of what is counted: its longest label in the entries' face.
const STAT_LABEL_WIDTH: f64 = 230.0;

impl Browser {
    /// The console's tabs and buttons, and the cookies' controls.
    pub(in crate::browser) fn console_tips(&self, tips: &mut Vec<(Rect, alloc::string::String)>) {
        if !self.console.open {
            return;
        }
        for &(tab, rect) in &self.console.tabs {
            let words = match tab {
                Tab::Entries(ConsoleTab::Network) => "Each exchange with the channel's server, and the push connection's events",
                Tab::Entries(ConsoleTab::Packets) => "Each teletext packet received",
                Tab::Entries(ConsoleTab::Pages) => "The pages the decoder holds",
                Tab::Entries(ConsoleTab::Log) => "The browser's log: each process's errors, warnings and notices, and how it was confined",
                Tab::Stats => "What this channel has sent and received since it opened: bytes, packets, sound, input events and pages",
                Tab::Cookies => "Cookies kept for each site and channel: delete or block them",
            };
            tips.push((rect, words.into()));
        }
        let minimise = if self.console.minimised() { "Restore the console's height" } else { "Minimise the console to its tabs" };
        tips.push((self.console.minimise, minimise.into()));
        let maximise = if self.console.maximised() { "Restore the console's height" } else { "Maximise the console: all the room the pages can spare" };
        tips.push((self.console.maximise, maximise.into()));
        let close =
            format!("Close the console; {} opens it again ({})", self.keys_named("the View menu", "the menu"), self.keys_named("Option+Cmd+I", "Ctrl+Shift+I"));
        tips.push((self.console.close, close));
        if self.console.tab == Tab::Cookies {
            self.cookies_tips(tips);
        } else if self.console.tab != Tab::Stats {
            tips.push((
                self.console.pause,
                if self.console.paused { "Scroll again, with what arrived meanwhile" } else { "Stop scrolling; what arrives waits" }.into(),
            ));
            tips.push((self.console.clear, "Empty every tab's entries".into()));
        }
    }

    /// The console, docked at the bottom in `area`.
    pub(in crate::browser) fn draw_console<C: Canvas + ?Sized>(&mut self, canvas: &mut C, area: Rect) {
        self.console.area = area;
        self.console.tabs.clear();
        canvas.fill(area, 0.0, PANEL);
        outline(canvas, area, RULE);
        let bar = Rect::new(area.x0 + PADDING, area.y0 + GAP, area.x1 - PADDING, area.y0 + GAP + BAR_HEIGHT);
        self.draw_console_bar(canvas, bar);
        if self.console.minimised() {
            return;
        }

        let list = Rect::new(area.x0 + PADDING, bar.y1 + GAP, area.x1 - PADDING, area.y1 - GAP);
        self.draw_console_entries(canvas, list);
    }

    /// The tab bar along `bar`: a tab for each of the console's tabs, the
    /// minimise, maximise and close buttons, Pause and Clear, and the Log
    /// tab's levels and search or the console's name.
    fn draw_console_bar<C: Canvas + ?Sized>(&mut self, canvas: &mut C, bar: Rect) {
        let mut x = bar.x0;
        for tab in TABS {
            let rect = Rect::new(x, bar.y0, x + TAB_WIDTH, bar.y1);
            let chosen = self.console.tab == tab;
            canvas.fill(rect, RADIUS, if chosen { SELECTED } else { BUTTON });
            border(canvas, rect, 1.0, if chosen { SELECTED } else { RULE });
            let label = match tab {
                Tab::Entries(tab) => format!("{} {}", tab.label(), self.console.entries.get(tab.index()).map_or(0, VecDeque::len)),
                Tab::Stats => "Stats".into(),
                Tab::Cookies => "Cookies".into(),
            };
            centred(canvas, rect, &label, ROW_STATUS, WHITE);
            self.console.tabs.push((tab, rect));
            x = rect.x1 + GAP;
        }
        self.draw_console_window_buttons(canvas, bar);
        // Pause and Clear are the entries'; the cookies' tab has its own controls, the figures none.
        let entries_shown = matches!(self.console.tab, Tab::Entries(_));
        let (clear, pause) = if entries_shown {
            let clear = Rect::new(self.console.minimise.x0 - GAP - BUTTON_WIDTH, bar.y0, self.console.minimise.x0 - GAP, bar.y1);
            (clear, Rect::new(clear.x0 - GAP - BUTTON_WIDTH, bar.y0, clear.x0 - GAP, bar.y1))
        } else {
            (Rect::default(), Rect::default())
        };
        (self.console.clear, self.console.pause) = (clear, pause);
        if entries_shown {
            for (rect, word, chosen) in
                [(self.console.pause, if self.console.paused { "Resume" } else { "Pause" }, self.console.paused), (self.console.clear, "Clear", false)]
            {
                canvas.fill(rect, RADIUS, if chosen { SELECTED } else { BUTTON });
                border(canvas, rect, 1.0, RULE);
                centred(canvas, rect, word, ROW_TITLE, WHITE);
            }
        }
        let leftmost = if entries_shown { self.console.pause.x0 } else { self.console.minimise.x0 };
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
    }

    /// Minimise, maximise and close at the right of `bar`, as a window's
    /// buttons lie, drawn with the overview's glyphs: minimise's chevron
    /// points up once it is minimised, as the overview's fold does, and
    /// maximise's square becomes the restore glyph once it is maximised.
    fn draw_console_window_buttons<C: Canvas + ?Sized>(&mut self, canvas: &mut C, bar: Rect) {
        let close = Rect::new(bar.x1 - WINDOW_BUTTON_WIDTH, bar.y0, bar.x1, bar.y1);
        let maximise = Rect::new(close.x0 - GAP - WINDOW_BUTTON_WIDTH, bar.y0, close.x0 - GAP, bar.y1);
        let minimise = Rect::new(maximise.x0 - GAP - WINDOW_BUTTON_WIDTH, bar.y0, maximise.x0 - GAP, bar.y1);
        (self.console.minimise, self.console.maximise, self.console.close) = (minimise, maximise, close);
        for rect in [minimise, maximise, close] {
            canvas.fill(rect, RADIUS, BUTTON);
            border(canvas, rect, 1.0, RULE);
        }
        chevron(canvas, minimise, !self.console.minimised(), WHITE);
        maximise_glyph(canvas, maximise, self.console.maximised(), BUTTON, WHITE);
        close_glyph(canvas, close, WHITE);
    }

    /// The tab's entries in `list`, the newest at the foot, or the
    /// cookies' tab.
    fn draw_console_entries<C: Canvas + ?Sized>(&mut self, canvas: &mut C, list: Rect) {
        if self.console.tab == Tab::Stats {
            self.draw_console_stats(canvas, list);
            return;
        }
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

    /// The Stats tab: each figure on a line, what is counted and then how much.
    fn draw_console_stats<C: Canvas + ?Sized>(&self, canvas: &mut C, list: Rect) {
        canvas.push_clip(list);
        if self.console.stats.is_empty() {
            left_centred(canvas, Rect::new(list.x0, list.y0, list.x1, list.y0 + LINE), 0.0, "Nothing yet", ENTRY, DIM);
        }
        for (row, stat) in self.console.stats.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "a panel's lines")]
            let top = list.y0 + row as f64 * LINE;
            let line = Rect::new(list.x0, top, list.x1, top + LINE);
            left_centred(canvas, line, 0.0, &stat.label, ENTRY, MUTED);
            left_centred(canvas, line, STAT_LABEL_WIDTH, &stat.value, ENTRY, WHITE);
        }
        canvas.pop_clip();
    }
}
