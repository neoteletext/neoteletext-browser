//! Closed tabs (requirement 20.9): each kept to reopen unless it is
//! private, and reopened where it was in its history, a closed window's
//! tabs together in a window of their own.

use std::collections::VecDeque;

use interface_core::library::{MAX_CLOSED, MAX_WINDOWS};
use protocol::{MAX_TAB_TITLE, clean};

use super::{ClosedTab, MAX_TABS, Pending, Session, Tab};

impl Session {
    /// Reopens a closed tab, the most recent or the one numbered `id`,
    /// where it was in its history, as the active tab (requirement 20.9).
    ///
    /// # Errors
    /// When there is none to reopen.
    pub fn reopen(&mut self, id: Option<u32>) -> Result<(), String> {
        let index = match id {
            Some(id) => self.closed.iter().position(|closed| closed.id == id),
            None => (!self.closed.is_empty()).then_some(0),
        }
        .ok_or("No closed tab to reopen")?;
        let window = self.closed.get(index).map_or(0, |closed| closed.window);
        if window == 0 {
            self.room_for_a_tab()?;
            let closed = self.closed.remove(index).ok_or("No closed tab to reopen")?;
            return self.reopen_tab(closed);
        }
        // A closed window: its tabs together, in a window of their own.
        let mut tabs: Vec<ClosedTab> = Vec::new();
        let mut kept = VecDeque::new();
        for closed in core::mem::take(&mut self.closed) {
            if closed.window == window { tabs.push(closed) } else { kept.push_back(closed) }
        }
        self.closed = kept;
        // They were kept the most recent first, the window's last tab first.
        tabs.reverse();
        let room = MAX_TABS.saturating_sub(self.tab_count());
        if room == 0 || self.window_count() >= MAX_WINDOWS {
            return Err(String::from("No room for another window"));
        }
        self.new_window(false)?;
        for closed in tabs.into_iter().take(room) {
            self.reopen_tab(closed)?;
        }
        self.active = 0;
        Ok(())
    }

    fn reopen_tab(&mut self, closed: ClosedTab) -> Result<(), String> {
        let address = closed.history.get(closed.position).map(|visit| visit.address.clone()).ok_or("No closed tab to reopen")?;
        let channel = self.channel_for(&address);
        self.active = self.tabs.len();
        let pending = Some(Pending { typed: false, drawn: false, reload: false });
        let private = self.private_window;
        self.tabs.push(Tab { history: closed.history, position: closed.position, channel, muted: closed.muted, private, pending });
        Ok(())
    }

    /// Keeps a closed tab to reopen, unless it is private.
    pub(super) fn keep_closed(&mut self, tab: Tab, window: u32) {
        if tab.private {
            return;
        }
        let title = clean(tab.address().map_or("", |address| address.host.as_str()), MAX_TAB_TITLE);
        let id = self.next_closed;
        self.next_closed = self.next_closed.wrapping_add(1).max(1);
        self.closed.push_front(ClosedTab { id, history: tab.history, position: tab.position, muted: tab.muted, title, window });
        self.closed.truncate(MAX_CLOSED);
    }
}
