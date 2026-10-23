//! The console's input: presses on its tabs, buttons, levels and search,
//! the keys of its search, and the wheel over its entries.

use alloc::collections::VecDeque;

use super::{LINE, Tab, level_index};
use crate::browser::{Browser, Key, Output};
use crate::editor::Editor;
use crate::geometry::Point;

impl Browser {
    /// A press on the console; returns whether it was the console's.
    pub(in crate::browser) fn console_press(&mut self, point: Point) -> bool {
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
    pub(in crate::browser) fn console_searching(&self) -> bool {
        self.console.open && self.console.searching
    }

    /// The log's search, while it has the keys.
    pub(in crate::browser) fn console_field(&mut self) -> Option<&mut Editor> {
        self.console_searching().then_some(&mut self.console.search)
    }

    /// A key while the log's search takes them; `None` when it does not.
    pub(in crate::browser) fn console_key(&mut self, key: Key<'_>, shift: bool) -> Option<bool> {
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
    pub(in crate::browser) fn console_scroll(&mut self, distance: f64) -> bool {
        if !self.console.open || !self.console.area.contains(self.pointer) {
            return false;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a few lines of wheel")]
        let lines = libm::ceil(distance.abs() / LINE) as usize;
        let most = self.console.shown().map_or(0, |entries| entries.len());
        self.console.back = if distance < 0.0 { self.console.back.saturating_add(lines).min(most) } else { self.console.back.saturating_sub(lines) };
        true
    }
}
