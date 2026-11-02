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
    Aspect,
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
        Self::Aspect,
        Self::BookmarkPage,
        #[cfg(feature = "phone-keyboard")]
        Self::Phone,
        Self::Console,
        Self::Remote,
        Self::FullScreen,
        Self::Settings,
    ];

    /// Its words, and its shortcut on macOS and elsewhere, in words: the
    /// browser's fonts have no key symbols. What it says of what is on or
    /// shown is `browser`'s state.
    fn words(self, browser: &Browser) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::NewTab => ("New tab", "Cmd+T", "Ctrl+T"),
            Self::NewWindow => ("New window", "Cmd+N", "Ctrl+N"),
            Self::NewPrivateTab => ("New private tab", "Shift+Cmd+N", "Ctrl+Shift+N"),
            Self::NewPrivateWindow => ("New private window", "", ""),
            Self::TabToNewWindow => ("Move tab to new window", "", ""),
            Self::Reopen => ("Reopen closed tab", "Shift+Cmd+T", "Ctrl+Shift+T"),
            Self::History => ("History", "Cmd+Y", "Ctrl+H"),
            Self::Bookmarks => ("Bookmarks", "Option+Cmd+B", "Ctrl+Shift+O"),
            Self::Bar if browser.bar_shown() => ("Hide bookmarks bar", "Shift+Cmd+B", "Ctrl+Shift+B"),
            Self::Bar => ("Show bookmarks bar", "Shift+Cmd+B", "Ctrl+Shift+B"),
            Self::Overview if browser.overview_shown() => ("Hide overview of all pages", "Shift+Cmd+E", "Ctrl+Shift+E"),
            Self::Overview => ("Show overview of all pages", "Shift+Cmd+E", "Ctrl+Shift+E"),
            // The wall of pages or the page alone (decision 126).
            Self::Wall if browser.wall.single() => ("Show wall of pages", "Cmd+G", "Ctrl+Shift+G"),
            Self::Wall => ("Show current page only", "Cmd+G", "Ctrl+Shift+G"),
            // The 4:3 aspect correction (REQ-737).
            Self::Aspect if browser.aspect_corrected() => ("Turn off aspect correction", "Shift+Cmd+A", "Ctrl+Shift+A"),
            Self::Aspect => ("Turn on aspect correction", "Shift+Cmd+A", "Ctrl+Shift+A"),
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
            Item::Aspect => self.toggle_aspect(),
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
        let mut y = area.y0;
        for &item in Item::ALL {
            let rect = Rect::new(area.x0, y, area.x1, y + ROW);
            if rect.contains(self.pointer) {
                canvas.fill(rect, 0.0, SELECTED);
            }
            // macOS's window buttons are its own: the browser draws none there.
            let (words, mac, elsewhere) = item.words(self);
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
    use crate::browser::fixture::{middle, set_aspect};
    use crate::browser::{Frame, StripState};
    use crate::settings::SettingChange;
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

    /// Opens the menu and says what its aspect correction item shows.
    fn aspect_words(browser: &mut Browser) -> (alloc::string::String, Rect) {
        let menu = browser.regions.menu;
        press(browser, menu, 0);
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, 1440.0, 900.0, 0);
        let rect = browser.menu.menu.iter().find(|(item, _)| *item == Item::Aspect).map(|(_, rect)| *rect).unwrap();
        let words = canvas.texts.iter().find(|(place, text)| rect.contains(middle(*place)) && text.contains("aspect")).map(|(_, text)| text.clone()).unwrap();
        (words, rect)
    }

    /// No item's words run into its shortcut, whichever way it says it: the
    /// overview's "Hide" line was read clear of its shortcut in a drawn
    /// window, so none is longer. The test canvas has no font to measure by.
    #[test]
    fn no_items_words_are_longer_than_those_seen_to_fit() {
        const LONGEST: usize = "Hide overview of all pages".len();
        let mut browser = browser();
        for on in [false, true] {
            set_aspect(&mut browser, on);
            for item in Item::ALL {
                let (words, _, _) = item.words(&browser);
                assert!(words.len() <= LONGEST, "{words}");
            }
        }
    }

    /// REQ-737 item 2: off by default; the item says what choosing it does,
    /// as the bar's and the overview's say, and asks the trusted core, whose
    /// answer alone turns it on.
    #[test]
    fn the_menu_shows_and_changes_the_aspect_correction() {
        let mut browser = browser();
        assert!(!browser.aspect_corrected(), "off in a new window");
        let (words, item) = aspect_words(&mut browser);
        assert_eq!(words, "Turn on aspect correction");
        press(&mut browser, item, 0);
        assert_eq!(browser.take_outputs(), [Output::Setting(SettingChange::Aspect(true))]);
        assert!(!browser.aspect_corrected(), "the trusted core keeps it");
        set_aspect(&mut browser, true);
        assert!(browser.aspect_corrected());
        let (words, item) = aspect_words(&mut browser);
        assert_eq!(words, "Turn off aspect correction");
        press(&mut browser, item, 0);
        assert_eq!(browser.take_outputs(), [Output::Setting(SettingChange::Aspect(false))]);
    }
}
