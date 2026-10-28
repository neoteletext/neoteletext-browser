//! The history panel's input: the keys of its search, the wheel, the row
//! under a press, and presses on its edges, controls, rows and row menu.

use alloc::string::String;
use alloc::vec::Vec;

use super::{RowAction, WHEEL_ROWS};
use crate::browser::{Browser, Key, Output};
use crate::geometry::Point;
use crate::library::{HistoryRequest, MAX_VIEW_ROWS};

impl Browser {
    /// A key while the panel is open; `None` when it is not the panel's.
    pub(in crate::browser) fn history_key(&mut self, key: Key<'_>, shift: bool) -> Option<bool> {
        if !self.history.open {
            return None;
        }
        if !self.history.focused {
            if key == Key::Escape {
                if self.history.clearing {
                    self.history.clearing = false;
                } else {
                    self.set_history_open(false);
                }
                return Some(true);
            }
            return None;
        }
        let editor = &mut self.history.query;
        let before = String::from(editor.text());
        match key {
            Key::Text(text) => editor.insert(text),
            Key::Backspace => editor.backspace(),
            Key::Delete => editor.delete(),
            Key::Left => editor.left(shift),
            Key::Right => editor.right(shift),
            Key::Home | Key::Up => editor.home(shift),
            Key::End | Key::Down => editor.end(shift),
            Key::Escape | Key::Tab | Key::Enter => self.unfocus_history(),
        }
        // The search follows each change as it is typed.
        if self.history.query.text() != before {
            self.history_query(0);
        }
        Some(true)
    }

    /// The wheel over the panel: further rows are asked for.
    pub(in crate::browser) fn history_scroll(&mut self, distance: f64) -> bool {
        if !self.history.open || !self.history.controls.panel.contains(self.pointer) {
            return false;
        }
        let Some(view) = &self.history.view else { return true };
        let first = usize::try_from(view.first).unwrap_or(0);
        let total = usize::try_from(view.total).unwrap_or(0);
        let shown = self.history.controls.rows.len().max(1);
        let next = if distance > 0.0 { first.saturating_add(WHEEL_ROWS).min(total.saturating_sub(shown)) } else { first.saturating_sub(WHEEL_ROWS) };
        if next != first {
            self.history_query(next);
        }
        true
    }

    /// The row whose title is at `point`, not its check box or menu.
    pub(in crate::browser) fn history_row_at(&self, point: Point) -> Option<u32> {
        if !self.history.open || self.history.menu.is_some() {
            return None;
        }
        let (number, check, menu, _) = self.history.controls.rows.iter().find(|(_, _, _, row)| row.contains(point))?;
        (!check.contains(point) && !menu.contains(point)).then_some(*number)
    }

    /// A press on the panel; returns whether it was the panel's. With
    /// `new_tab`, a row opens in a new tab.
    /// Whether `point` is on the open history panel.
    pub(in crate::browser) fn over_history(&self, point: Point) -> bool {
        self.history.open && self.history.controls.panel.contains(point)
    }

    pub(in crate::browser) fn history_press(&mut self, point: Point, new_tab: bool) -> bool {
        if !self.history.open {
            return false;
        }
        // An edge's grip reaches a little outside the panel.
        if self.history_edge_press(point) {
            self.history.menu = None;
            return true;
        }
        let controls = &self.history.controls;
        if !controls.panel.contains(point) {
            return false;
        }
        // A row's open menu takes the press; anywhere else closes it.
        if let Some(number) = self.history.menu.take() {
            match controls.menu.iter().find(|(_, rect)| rect.contains(point)).map(|(action, _)| *action) {
                Some(RowAction::MoreFromSite) => self.outputs.push(Output::History(HistoryRequest::MoreFromSite(number))),
                Some(RowAction::Remove) => self.outputs.push(Output::History(HistoryRequest::Remove(alloc::vec![number]))),
                Some(RowAction::RemoveBookmark) => self.outputs.push(Output::History(HistoryRequest::RemoveBookmark(number))),
                None => {}
            }
            return true;
        }
        if controls.close.contains(point) {
            self.set_history_open(false);
            return true;
        }
        if controls.search.contains(point) {
            if !self.history.focused {
                self.history.focused = true;
                self.outputs.push(Output::TextInput(true));
            }
            return true;
        }
        self.unfocus_history();
        let controls = &self.history.controls;
        if controls.clear.contains(point) {
            self.history.clearing = !self.history.clearing;
        } else if controls.cancel.contains(point) {
            self.history.clearing = false;
        } else if controls.with_history.contains(point) {
            self.history.clears.history = !self.history.clears.history;
        } else if controls.with_cookies.contains(point) {
            self.history.clears.cookies = !self.history.clears.cookies;
        } else if let Some(&(range, _)) = controls.ranges.iter().find(|(_, rect)| rect.contains(point)) {
            self.history.clearing = false;
            self.history.selected.clear();
            if self.history.clears.history {
                self.outputs.push(Output::History(HistoryRequest::Clear(range)));
            }
            if self.history.clears.cookies {
                self.outputs.push(Output::History(HistoryRequest::ClearCookies(range)));
            }
        } else if controls.remove_selected.contains(point) {
            let numbers: Vec<u32> = self.history.selected.iter().copied().take(MAX_VIEW_ROWS).collect();
            self.history.selected.clear();
            self.outputs.push(Output::History(HistoryRequest::Remove(numbers)));
        } else if let Some(&(number, _)) = controls.closed.iter().find(|(_, rect)| rect.contains(point)) {
            self.outputs.push(Output::History(HistoryRequest::Reopen(Some(number))));
        } else if let Some(&(number, check, menu, _)) = controls.rows.iter().find(|(_, _, _, row)| row.contains(point)) {
            if menu.contains(point) {
                self.history.menu = Some(number);
            } else if check.contains(point) {
                if !self.history.selected.remove(&number) {
                    self.history.selected.insert(number);
                }
            } else {
                self.outputs.push(Output::History(HistoryRequest::Open { number, new_tab }));
                if !new_tab {
                    self.set_history_open(false);
                }
            }
        }
        true
    }
}
