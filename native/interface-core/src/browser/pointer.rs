//! The pointer: where it is and the shape it takes, and the presses,
//! releases and wheel turns, each given to the parts of the window in
//! the order they lie over each other.

use crate::browser::{Browser, LINE_HEIGHT, Output};
use crate::geometry::Point;

mod press;

/// The shape the pointer should take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    /// The ordinary pointer.
    Arrow,
    /// Over something a click acts on.
    Hand,
    /// Over a link that leaves the channel: the shape of a shortcut.
    External,
    /// Over the divider between the page list and the page area.
    ResizeColumn,
    /// Over the divider between the pages and their sub-pages.
    ResizeRow,
}

impl Browser {
    /// The pointer moved to `x`, `y` in logical pixels; returns whether
    /// the window needs drawing again.
    pub fn pointer_moved(&mut self, x: f64, y: f64, now: u64) -> bool {
        self.pointer = Point::new(x, y);
        if self.remote_moved(self.pointer) || self.divider_moved(self.pointer) || self.console_grabber_moved(self.pointer) {
            return true;
        }
        let subpaged = self.subpages_moved(self.pointer);
        if subpaged && self.over_subpage_divider() {
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
        let tabbed = self.tabs_moved(self.pointer, now);
        let listed = self.lists_hovered();
        let full_screen = self.full_screen_moved(self.pointer, self.body.width(), now);
        dragged || hovered || preview_changed || linked || tipped || tabbed || full_screen || listed || subpaged
    }

    /// The pointer's shape where it is: the column-resize shape over the
    /// divider, a hand over a page a click would zoom or choose and over
    /// the overview's fold button, the arrow elsewhere.
    #[must_use]
    pub fn cursor(&self) -> Cursor {
        if self.over_divider() {
            return Cursor::ResizeColumn;
        }
        if self.over_subpage_divider() || self.over_console_grabber() {
            return Cursor::ResizeRow;
        }
        if self.subpages.hovered.is_some() {
            return Cursor::Hand;
        }
        #[cfg(feature = "links")]
        if let Some(cursor) = self.link_cursor() {
            return cursor;
        }
        let over_toggle = self.regions.overview_toggle.contains(self.pointer) || self.hovered_row.is_some() || self.hovered_bank.is_some();
        if self.wall.over_page() || self.previewed.is_some() || over_toggle { Cursor::Hand } else { Cursor::Arrow }
    }

    /// The page row and the magazine under the pointer, highlighted and
    /// shown with the hand, as a click on them chooses; returns whether
    /// either changed.
    fn lists_hovered(&mut self) -> bool {
        let point = self.pointer;
        let row = self.regions.list.contains(point).then(|| self.regions.rows.iter().find(|(rect, _)| rect.contains(point)).map(|&(_, index)| index)).flatten();
        let bank = self.regions.banks.iter().position(|rect| rect.contains(point));
        let changed = row != self.hovered_row || bank != self.hovered_bank;
        self.hovered_row = row;
        self.hovered_bank = bank;
        changed
    }

    /// The main button was released.
    pub fn pointer_released(&mut self) {
        self.remote_released();
        self.divider_released();
        self.console_grabber_released();
        self.subpages_released();
        self.tabs_released();
        self.release_back_or_forward();
        let new_tab = self.press_new_tab;
        self.bookmarks_released(new_tab);
        self.wall.pointer_released();
        #[cfg(feature = "links")]
        self.links_released();
        // A button the channel saw pressed is released to it.
        #[cfg(feature = "interaction")]
        self.terminal_button(neoteletext_input::event::MouseAction::Release, 0);
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
        if self.regions.subpage_list.contains(self.pointer) {
            self.subpages.scroll = (self.subpages.scroll + distance).max(0.0);
            return true;
        }
        if self.wall.contains(self.pointer) {
            self.wall.scroll_by(distance);
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::fixture::{HEIGHT, WIDTH, browser, press_at};
    use crate::browser::{Key, Modifiers};

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
}
