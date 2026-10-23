//! The pointer: where it is and the shape it takes, and the presses,
//! releases and wheel turns, each given to the parts of the window in
//! the order they lie over each other.

use crate::browser::{Browser, LINE_HEIGHT, Output, WindowAction};
use crate::canvas::Canvas;
use crate::channel::page_index;
use crate::geometry::Point;
use crate::wall::CellClick;

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

impl Browser {
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
    pub fn pointer_pressed<C: Canvas + ?Sized>(&mut self, canvas: &mut C, extend: bool, now: u64) -> bool {
        self.tips_pressed();
        // A press means the button was let go since the last: a drag whose
        // release never came, as one let go outside every window, is over.
        self.tabs_cancel();
        // Shift opens what a press opens in a new tab.
        self.press_new_tab = extend;
        if let Some(taken) = self.press_overlays(extend, now) {
            return taken;
        }
        if let Some(taken) = self.press_strip(canvas, extend, now) {
            return taken;
        }
        self.press_body(now).unwrap_or(false)
    }

    /// A press on what lies over the window: the drop-downs, the tab
    /// strip, the star's popover, the panels, the strip's question, the
    /// certificate-error screen and the remote. `None` when it was on none.
    fn press_overlays(&mut self, extend: bool, now: u64) -> Option<bool> {
        // The drop-downs and the star's popover are over everything; the
        // panels over everything below the strip.
        if self.popups_press(self.pointer, extend) || self.tabs_press(self.pointer) || self.star_press(self.pointer) {
            return Some(true);
        }
        if self.history_press(self.pointer, extend) || self.bookmarks_press(self.pointer) {
            return Some(true);
        }
        if self.settings_press(self.pointer) {
            return Some(true);
        }
        if self.console_press(self.pointer) {
            return Some(true);
        }
        #[cfg(feature = "phone-keyboard")]
        if self.phone_press(self.pointer) {
            return Some(true);
        }
        // The strip's question, drawn over the address field.
        #[cfg(feature = "links")]
        if self.link_press(now) {
            return Some(true);
        }
        if self.refused_press(self.pointer) {
            return Some(true);
        }
        if self.remote_press(self.pointer, now) {
            return Some(true);
        }
        None
    }

    /// A press on the trusted strip: the address field, Back and Forward,
    /// the gear, the menu, the fixed buttons, the bookmarks bar and the
    /// empty strip, which drags the window. `None` when it was on none.
    fn press_strip<C: Canvas + ?Sized>(&mut self, canvas: &mut C, extend: bool, now: u64) -> Option<bool> {
        let point = self.pointer;
        let regions = &self.regions;
        if regions.field.contains(point) {
            if self.editing {
                let at = self.offset_at(canvas, point.x);
                self.editor.place(at, extend);
            } else {
                self.start_editing();
            }
            return Some(true);
        }
        self.stop_editing();
        let regions = &self.regions;
        // Back and Forward act when released, or open their lists when held.
        for (rect, forward, enabled) in [(regions.back, false, self.strip.back), (regions.forward, true, self.strip.forward)] {
            if rect.contains(point) {
                if enabled {
                    self.hold_back_or_forward(forward, now);
                }
                return Some(true);
            }
        }
        if regions.gear.contains(point) {
            self.toggle_settings();
            return Some(true);
        }
        if regions.menu.contains(point) {
            self.toggle_menu();
            return Some(true);
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
            return Some(true);
        }
        if self.bar_press(point, extend) {
            return Some(true);
        }
        let regions = &self.regions;
        if regions.strip.contains(point) {
            self.outputs.push(Output::Window(WindowAction::Drag));
            return Some(false);
        }
        None
    }

    /// A press below the strip: the banks, the list, the overview, the
    /// page's controls, the wall's controls and a page or its cell.
    /// `None` when it was on none.
    fn press_body(&mut self, now: u64) -> Option<bool> {
        let point = self.pointer;
        let regions = &self.regions;
        if let Some(bank) = regions.banks.iter().position(|rect| rect.contains(point)) {
            self.choose_bank(bank);
            return Some(true);
        }
        if regions.list.contains(point) {
            if let Some(&(_, index)) = regions.rows.iter().find(|(rect, _)| rect.contains(point)) {
                self.select_page(index, true);
            }
            return Some(true);
        }
        if regions.overview_toggle.contains(point) {
            self.toggle_overview();
            return Some(true);
        }
        if regions.cells.contains(point) {
            if let Some(index) = self.cell_index(point)
                && matches!(self.channel.pages.get(index), Some(Some(_)))
            {
                self.select_page(index, true);
            }
            return Some(true);
        }
        if self.controls_press(point) {
            return Some(true);
        }
        if self.wall.pointer_pressed(point, now) {
            return Some(true);
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
                return Some(self.terminal_button(neoteletext_input::event::MouseAction::Press, 0));
            }
            if self.wall.zoomed().is_some() {
                let version = self.versions.get(&click.key).copied().unwrap_or_default();
                self.outputs.push(Output::Cell(CellClick { version, ..click }));
            } else {
                self.wall.zoom(click.key, now);
            }
            return Some(true);
        }
        None
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
