//! The console drawn: its tabs and buttons, the Log tab's levels and
//! search, the cookies' tab or the entries; and its tips.

use alloc::format;
use alloc::vec::Vec;

use super::{
    BAR_HEIGHT, BUTTON_WIDTH, CLOSE_WIDTH, ENTRY, GAP, LEVEL_WIDTH, LINE, MILLISECONDS_PER_SECOND, PADDING, RADIUS, SEARCH_WIDTH, TAB_WIDTH, TABS, TIME_WIDTH,
    Tab, level_colour, level_index,
};
use crate::browser::Browser;
use crate::browser::widgets::field;
use crate::canvas::Canvas;
use crate::console::{ConsoleTab, Level};
use crate::geometry::Rect;
use crate::theme::{BUTTON, DIM, HEADING, MUTED, PANEL, ROW_STATUS, ROW_TITLE, RULE, SELECTED, WHITE};
use crate::ui::{border, centred, left_centred, outline};

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
    pub(in crate::browser) fn draw_console<C: Canvas + ?Sized>(&mut self, canvas: &mut C, area: Rect) {
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
