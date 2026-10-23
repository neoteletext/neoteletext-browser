//! The strip's drop-downs (`history-and-bookmarks.md` sections 2.9, 2.10
//! and 6): the menu button's menu; Back's and Forward's lists, opened by
//! holding the button or a secondary click (requirement 20.10); and the
//! address field's suggestions with inline completion (requirement 20.11).
//! Each shows what the trusted core sent and asks by number.

use crate::browser::Browser;
use crate::canvas::Canvas;
use crate::geometry::Point;

/// What the three drop-downs share: a row's height, the gap at a row's
/// ends, and the gap before a menu item's shortcut.
pub(super) const ROW: f64 = 30.0;
pub(super) const GAP: f64 = 8.0;
pub(super) const SHORTCUT_GAP: f64 = 12.0;

impl Browser {
    /// Every drop-down, over the rest.
    pub(super) fn draw_popups<C: Canvas + ?Sized>(&mut self, canvas: &mut C) {
        self.draw_suggestions(canvas);
        self.draw_tab_list(canvas);
        self.draw_menu(canvas);
    }

    /// A press on any drop-down; returns whether it took it.
    pub(super) fn popups_press(&mut self, point: Point, new_tab: bool) -> bool {
        self.menu_press(point) || self.tab_list_press(point) || self.suggestion_press(point, new_tab)
    }

    /// Whether the menu or a Back or Forward list is open.
    pub(super) fn popup_open(&self) -> bool {
        self.menu.menu_open || self.tab_lists.tab_list.is_some()
    }

    /// Whether a drop-down is open to close with Escape; closes it.
    pub(super) fn close_popup(&mut self) -> bool {
        if self.menu.menu_open {
            self.menu.menu_open = false;
            return true;
        }
        self.tab_lists.tab_list.take().is_some()
    }
}
