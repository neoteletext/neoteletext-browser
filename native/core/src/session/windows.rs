//! The windows set aside (`multiple-windows.md`): a window's tabs while
//! another is at hand, and the session's moves between windows.

use interface_core::library::{MAX_WINDOWS, WindowGeometry};

use super::{FIRST_WINDOW, Session, Tab};

/// A window set aside while another is at hand (`multiple-windows.md`):
/// its tabs, which is active, its place, and whether it is private.
pub struct Window {
    pub id: u32,
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub geometry: Option<WindowGeometry>,
    pub private: bool,
}

impl Session {
    /// How many windows are open.
    #[must_use]
    pub fn window_count(&self) -> usize {
        self.others.len().saturating_add(1)
    }

    /// Every window's number, in the order they opened.
    #[must_use]
    pub fn window_ids(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.others.iter().map(|window| window.id).chain([self.window]).collect();
        ids.sort_unstable();
        ids
    }

    /// The window holding `channel`'s tab.
    #[must_use]
    pub fn window_of(&self, channel: u32) -> Option<u32> {
        if self.tabs.iter().any(|tab| tab.channel.id == channel) {
            return Some(self.window);
        }
        self.others.iter().find(|window| window.tabs.iter().any(|tab| tab.channel.id == channel)).map(|window| window.id)
    }

    /// Every tab, in every window.
    pub fn all_tabs(&self) -> impl Iterator<Item = &Tab> {
        self.tabs.iter().chain(self.others.iter().flat_map(|window| window.tabs.iter()))
    }

    /// Every tab, in every window, to change.
    pub fn all_tabs_mut(&mut self) -> impl Iterator<Item = &mut Tab> {
        self.tabs.iter_mut().chain(self.others.iter_mut().flat_map(|window| window.tabs.iter_mut()))
    }

    /// Makes window `id` the one at hand; false when there is none.
    pub fn switch_to(&mut self, id: u32) -> bool {
        if id == self.window {
            return true;
        }
        let Some(index) = self.others.iter().position(|window| window.id == id) else { return false };
        let mut other = self.others.swap_remove(index);
        core::mem::swap(&mut self.tabs, &mut other.tabs);
        core::mem::swap(&mut self.active, &mut other.active);
        core::mem::swap(&mut self.geometry, &mut other.geometry);
        core::mem::swap(&mut self.private_window, &mut other.private);
        other.id = core::mem::replace(&mut self.window, id);
        self.others.push(other);
        true
    }

    /// Sets the window at hand aside and makes a new one, with no tab yet,
    /// the one at hand; returns its number.
    ///
    /// # Errors
    /// When [`MAX_WINDOWS`] are open.
    pub fn new_window(&mut self, private: bool) -> Result<u32, String> {
        if self.window_count() >= MAX_WINDOWS {
            return Err(format!("At most {MAX_WINDOWS} windows are open at once"));
        }
        let id = self.next_window;
        self.next_window = self.next_window.wrapping_add(1).max(FIRST_WINDOW);
        let parked = Window {
            id: core::mem::replace(&mut self.window, id),
            tabs: core::mem::take(&mut self.tabs),
            active: core::mem::take(&mut self.active),
            geometry: self.geometry.take(),
            private: core::mem::replace(&mut self.private_window, private),
        };
        self.others.push(parked);
        Ok(id)
    }

    /// Closes the window at hand, its tabs kept together to reopen as a
    /// window unless private, and makes the most recent other window the
    /// one at hand. Returns the closed tabs' channels; none, and nothing
    /// closed, when it is the only window.
    pub fn close_window(&mut self) -> Vec<u32> {
        let Some(next) = self.others.last().map(|window| window.id) else { return Vec::new() };
        let window = self.window;
        let tabs = core::mem::take(&mut self.tabs);
        let channels = tabs.iter().map(|tab| tab.channel.id).collect();
        for tab in tabs {
            self.keep_closed(tab, window);
        }
        self.active = 0;
        self.switch_to(next);
        self.others.retain(|other| other.id != window);
        channels
    }

    /// Takes the tab at `index` out of the window at hand, as a close does,
    /// to put in another window; none when there is no such tab.
    pub fn take_tab(&mut self, index: usize) -> Option<Tab> {
        if index >= self.tabs.len() {
            return None;
        }
        let tab = self.tabs.remove(index);
        if self.active >= self.tabs.len() || index < self.active {
            self.active = self.active.saturating_sub(1);
        }
        Some(tab)
    }

    /// Puts `tab` into the window at hand at `index`, or its end, as its
    /// active tab.
    pub fn put_tab(&mut self, tab: Tab, index: usize) {
        let index = index.min(self.tabs.len());
        self.tabs.insert(index, tab);
        self.active = index;
    }
}
