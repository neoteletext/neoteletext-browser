//! The menu button's menu (`history-and-bookmarks.md` section 6): new
//! tabs and windows, history, bookmarks, the bar, the console, the remote
//! and settings, each with its shortcut in words.

use alloc::vec::Vec;

use crate::browser::popups::{GAP, ROW, SHORTCUT_GAP};
use crate::browser::{Browser, Output};
use crate::canvas::Canvas;
use crate::geometry::{Point, Rect};
use crate::library::{BarRequest, HistoryRequest};
use crate::text::{HAIRLINE, border, left_centred, right_centred};
use crate::theme::{BLACK, MUTED, ROW_STATUS, ROW_TITLE, RULE, SELECTED, WHITE};

const MENU_WIDTH: f64 = 300.0;

/// What the menu holds (section 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    NewTab,
    NewWindow,
    NewPrivateTab,
    NewPrivateWindow,
    TabToNewWindow,
    Reopen,
    History,
    Bookmarks,
    Bar,
    Overview,
    Wall,
    BookmarkPage,
    #[cfg(feature = "phone-keyboard")]
    Phone,
    Console,
    Remote,
    FullScreen,
    Settings,
}

impl Item {
    const ALL: &[Self] = &[
        Self::NewTab,
        Self::NewWindow,
        Self::NewPrivateTab,
        Self::NewPrivateWindow,
        Self::TabToNewWindow,
        Self::Reopen,
        Self::History,
        Self::Bookmarks,
        Self::Bar,
        Self::Overview,
        Self::Wall,
        Self::BookmarkPage,
        #[cfg(feature = "phone-keyboard")]
        Self::Phone,
        Self::Console,
        Self::Remote,
        Self::FullScreen,
        Self::Settings,
    ];

    /// Its words, and its shortcut on macOS and elsewhere, in words: the
    /// browser's fonts have no key symbols.
    fn words(self, bar_shown: bool, overview_shown: bool, single: bool) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::NewTab => ("New tab", "Cmd+T", "Ctrl+T"),
            Self::NewWindow => ("New window", "Cmd+N", "Ctrl+N"),
            Self::NewPrivateTab => ("New private tab", "Shift+Cmd+N", "Ctrl+Shift+N"),
            Self::NewPrivateWindow => ("New private window", "", ""),
            Self::TabToNewWindow => ("Move tab to new window", "", ""),
            Self::Reopen => ("Reopen closed tab", "Shift+Cmd+T", "Ctrl+Shift+T"),
            Self::History => ("History", "Cmd+Y", "Ctrl+H"),
            Self::Bookmarks => ("Bookmarks", "Option+Cmd+B", "Ctrl+Shift+O"),
            Self::Bar if bar_shown => ("Hide bookmarks bar", "Shift+Cmd+B", "Ctrl+Shift+B"),
            Self::Bar => ("Show bookmarks bar", "Shift+Cmd+B", "Ctrl+Shift+B"),
            Self::Overview if overview_shown => ("Hide overview of all pages", "Shift+Cmd+E", "Ctrl+Shift+E"),
            Self::Overview => ("Show overview of all pages", "Shift+Cmd+E", "Ctrl+Shift+E"),
            // The wall of pages or the page alone (decision 126).
            Self::Wall if single => ("Show wall of pages", "Cmd+G", "Ctrl+Shift+G"),
            Self::Wall => ("Show current page only", "Cmd+G", "Ctrl+Shift+G"),
            Self::BookmarkPage => ("Bookmark this page", "Cmd+D", "Ctrl+D"),
            #[cfg(feature = "phone-keyboard")]
            Self::Phone => ("Phone keyboard", "", ""),
            Self::Console => ("Developer console", "Option+Cmd+I", "Ctrl+Shift+I"),
            Self::Remote => ("Remote control", "Shift+Cmd+R", "Ctrl+Shift+R"),
            // The page alone (decision 85), by the system's own key.
            Self::FullScreen => ("Full screen", "Ctrl+Cmd+F", "F11"),
            Self::Settings => ("Settings", "Cmd+,", "Ctrl+,"),
        }
    }
}

/// The menu's state in the browser.
#[derive(Debug, Default)]
pub(super) struct MenuUi {
    menu_open: bool,
    menu: Vec<(Item, Rect)>,
    menu_area: Rect,
}

impl MenuUi {
    /// Whether the menu is open.
    pub(super) fn is_open(&self) -> bool {
        self.menu_open
    }

    /// Closes the menu; whether it was open.
    pub(super) fn close(&mut self) -> bool {
        core::mem::take(&mut self.menu_open)
    }
}

impl Browser {
    /// The menu button: its menu opens, or closes.
    pub(super) fn toggle_menu(&mut self) {
        self.menu.menu_open = !self.menu.menu_open;
    }

    /// A press while the menu is open: an item acts; anywhere else closes it.
    /// Whether `point` is on the open menu.
    pub(super) fn over_menu(&self, point: Point) -> bool {
        self.menu.menu_open && self.menu.menu_area.contains(point)
    }

    pub(super) fn menu_press(&mut self, point: Point) -> bool {
        if !self.menu.menu_open {
            return false;
        }
        self.menu.menu_open = false;
        let Some(&(item, _)) = self.menu.menu.iter().find(|(_, rect)| rect.contains(point)) else {
            // The menu button pressed again only closes it.
            return self.regions.menu.contains(point) || self.menu.menu_area.contains(point);
        };
        match item {
            Item::NewTab => self.outputs.push(Output::NewTab),
            Item::NewWindow => self.outputs.push(Output::NewWindow { private: false }),
            Item::NewPrivateTab => self.outputs.push(Output::NewPrivateTab),
            Item::NewPrivateWindow => self.outputs.push(Output::NewWindow { private: true }),
            Item::TabToNewWindow => self.outputs.push(Output::TabToNewWindow),
            Item::Reopen => self.outputs.push(Output::History(HistoryRequest::Reopen(None))),
            Item::History => self.set_history_open(true),
            Item::Bookmarks => {
                if !self.bookmarks_open() {
                    self.toggle_bookmarks();
                }
            }
            Item::Bar => self.outputs.push(Output::Bar(BarRequest::Show(!self.bar_shown()))),
            Item::Overview => self.toggle_overview_shown(),
            Item::Wall => {
                // The last frame's time: a menu's press is not given the clock.
                let now = self.remote_time();
                self.toggle_wall(now);
            }
            Item::BookmarkPage => self.outputs.push(Output::Bookmark),
            #[cfg(feature = "phone-keyboard")]
            Item::Phone => self.toggle_phone_panel(),
            Item::Console => self.toggle_console(),
            Item::Remote => self.toggle_remote(self.remote_time()),
            Item::FullScreen => {
                // The last frame's time: a menu's press is not given the clock.
                let now = self.remote_time();
                self.toggle_full_screen(now);
            }
            Item::Settings => {
                if !self.settings_open() {
                    self.toggle_settings();
                }
            }
        }
        true
    }

    pub(super) fn draw_menu<C: Canvas + ?Sized>(&mut self, canvas: &mut C) {
        self.menu.menu.clear();
        if !self.menu.menu_open {
            self.menu.menu_area = Rect::ZERO;
            return;
        }
        let anchor = self.regions.menu;
        #[allow(clippy::cast_precision_loss, reason = "under twenty items")]
        let area = Rect::new(anchor.x1 - MENU_WIDTH, anchor.y1 + HAIRLINE, anchor.x1, anchor.y1 + HAIRLINE + ROW * Item::ALL.len() as f64);
        canvas.fill(area, 0.0, BLACK);
        border(canvas, area, HAIRLINE, RULE);
        let bar_shown = self.bar_shown();
        let mut y = area.y0;
        for &item in Item::ALL {
            let rect = Rect::new(area.x0, y, area.x1, y + ROW);
            if rect.contains(self.pointer) {
                canvas.fill(rect, 0.0, SELECTED);
            }
            // macOS's window buttons are its own: the browser draws none there.
            let (words, mac, elsewhere) = item.words(bar_shown, self.overview_shown(), self.wall.single());
            let shortcut = if self.frame.window_buttons { elsewhere } else { mac };
            left_centred(canvas, rect, GAP * 2.0, words, ROW_TITLE, WHITE);
            right_centred(canvas, rect, SHORTCUT_GAP, shortcut, ROW_STATUS, MUTED);
            self.menu.menu.push((item, rect));
            y += ROW;
        }
        self.menu.menu_area = area;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::fixture::middle;
    use crate::browser::{Frame, StripState};
    use crate::test_canvas::Recorder;

    fn browser() -> Browser {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
        browser.set_strip(StripState { address: "https://news.example/".into(), back: true, forward: true, ..StripState::default() });
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        browser
    }

    fn press(browser: &mut Browser, rect: Rect, now: u64) {
        let point = middle(rect);
        browser.pointer_moved(point.x, point.y, now);
        browser.pointer_pressed(&mut Recorder::new(), false, now);
    }

    /// Section 6: the menu holds history, bookmarks and private tabs.
    #[test]
    fn the_menu_holds_history_and_bookmarks() {
        let mut browser = browser();
        let menu = browser.regions.menu;
        press(&mut browser, menu, 0);
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        let private = browser.menu.menu.iter().find(|(item, _)| *item == Item::NewPrivateTab).map(|(_, rect)| *rect).unwrap();
        press(&mut browser, private, 0);
        assert_eq!(browser.take_outputs(), [Output::NewPrivateTab]);
        press(&mut browser, menu, 0);
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        let history = browser.menu.menu.iter().find(|(item, _)| *item == Item::History).map(|(_, rect)| *rect).unwrap();
        press(&mut browser, history, 0);
        assert_eq!(browser.take_outputs(), [Output::History(HistoryRequest::Panel(true))]);
        assert!(browser.history_open());
    }
}
