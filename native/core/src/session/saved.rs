//! The session kept and restored (`history-and-bookmarks.md` section 15,
//! requirements 20.21 to 20.23): what is written after every change, what
//! opens at start, and the entries removed with history.

use interface_core::library::{MAX_CLOSED, WindowGeometry};
use protocol::{MAX_TAB_TITLE, clean};

use neoteletext::address::parse;
use neoteletext::restore::{self, Saved};

use super::tab::without;
use super::{ClosedTab, MAX_TABS, Pending, Session, Tab, Visit};

/// A kept tab's list as entries, when every address is still accepted and
/// its place is in it; a refusal is logged.
fn kept_history(kept: &restore::Tab) -> Option<Vec<Visit>> {
    let history: Result<Vec<Visit>, String> = kept
        .entries
        .iter()
        .map(|entry| Ok(Visit { address: parse(&entry.address)?, by_page: entry.by_page, title: clean(&entry.title, restore::MAX_TITLE), at: entry.at }))
        .collect();
    match history {
        Ok(history) if kept.position < history.len() => Some(history),
        Ok(_) => None,
        Err(reason) => {
            eprintln!("neoteletext: a tab of the last session is not restored: {reason}");
            None
        }
    }
}

impl Session {
    /// The session to keep (`history-and-bookmarks.md` section 15.1): every
    /// tab but private ones, each list cut to the entries nearest its place,
    /// and the active tab's place among them.
    #[must_use]
    pub fn saved(&self) -> Saved {
        let entry = |visit: &Visit| restore::Entry {
            by_page: visit.by_page,
            at: visit.at,
            address: visit.address.text.clone(),
            title: clean(&visit.title, restore::MAX_TITLE),
        };
        let window = |geometry: Option<WindowGeometry>, tabs: &[Tab], active: usize| {
            let kept: Vec<(usize, &Tab)> = tabs.iter().enumerate().filter(|(_, tab)| !tab.private).collect();
            // A private active tab: the kept tab before it, or the first.
            let active = kept.iter().filter(|(index, _)| *index <= active).count().saturating_sub(1);
            let tabs = kept.iter().map(|(_, tab)| restore::Tab::trimmed(tab.muted, tab.position, tab.history.iter().map(entry).collect())).collect();
            restore::Window { geometry, active, tabs }
        };
        // The windows in the order they opened, the one at hand among them.
        let mut windows: Vec<(u32, restore::Window)> = self.others.iter().map(|other| (other.id, window(other.geometry, &other.tabs, other.active))).collect();
        windows.push((self.window, window(self.geometry, &self.tabs, self.active)));
        windows.sort_by_key(|(id, _)| *id);
        let windows = windows.into_iter().map(|(_, window)| window).filter(|window| !window.tabs.is_empty()).collect();
        let closed = self
            .closed
            .iter()
            .map(|tab| restore::Tab { window: tab.window, ..restore::Tab::trimmed(tab.muted, tab.position, tab.history.iter().map(entry).collect()) })
            .collect();
        Saved { windows, closed }
    }

    /// Opens the kept session's tabs after any open, each at its place in
    /// its list and waiting for its page to draw as a navigation does, and
    /// makes its active tab active (section 15.3). A tab whose address the
    /// address rules now refuse is not opened. Returns how many opened.
    pub fn restore(&mut self, saved: &Saved) -> usize {
        let before = self.tab_count();
        // The first window's tabs join the window at hand; each other kept
        // window opens as a window of its own, and the first stays at hand.
        let at_hand = self.window;
        for (index, kept_window) in saved.windows.iter().enumerate() {
            if index > 0 && (self.tab_count() >= MAX_TABS || self.new_window(false).is_err()) {
                break;
            }
            if index > 0 || self.geometry.is_none() {
                self.geometry = kept_window.geometry;
            }
            let first = self.tabs.len();
            let mut active = None;
            for (place, kept) in kept_window.tabs.iter().enumerate() {
                if self.tab_count() >= MAX_TABS {
                    break;
                }
                let Some(history) = kept_history(kept) else { continue };
                let Some(address) = history.get(kept.position).map(|visit| visit.address.clone()) else { continue };
                let channel = self.channel_later(&address);
                if place == kept_window.active {
                    active = Some(self.tabs.len());
                }
                let pending = Some(Pending { typed: false, drawn: false, reload: false });
                self.tabs.push(Tab { history, position: kept.position, channel, muted: kept.muted, private: false, pending });
            }
            if let Some(active) = active.or_else(|| (self.tabs.len() > first).then_some(first)) {
                self.active = active;
            }
            // A kept window none of whose tabs could open is not opened.
            if index > 0 && self.tabs.is_empty() {
                self.close_window();
            }
        }
        self.switch_to(at_hand);
        // The recently closed tabs, after any closed since start.
        for kept in &saved.closed {
            if self.closed.len() >= MAX_CLOSED {
                break;
            }
            let Some(history) = kept_history(kept) else { continue };
            let title = clean(history.get(kept.position).map_or("", |visit| visit.address.host.as_str()), MAX_TAB_TITLE);
            let id = self.next_closed;
            self.next_closed = self.next_closed.wrapping_add(1).max(1);
            self.closed.push_back(ClosedTab { id, history, position: kept.position, muted: kept.muted, title, window: kept.window });
        }
        self.tab_count().saturating_sub(before)
    }

    /// Removes the entries `gone` names from the tabs' lists and the closed
    /// tabs', after history was removed, as Chrome does
    /// (`browsing_data::RemoveNavigationEntries`): an open tab keeps the
    /// entry it is at; a closed tab whose entry it was at goes whole; a
    /// private tab is not touched. Returns whether any list changed.
    pub fn remove_entries(&mut self, gone: impl Fn(&Visit) -> bool) -> bool {
        let mut changed = false;
        for tab in self.all_tabs_mut().filter(|tab| !tab.private) {
            let (before, at) = (tab.history.len(), tab.position);
            let (history, position) = without(core::mem::take(&mut tab.history), at, |index, visit| index != at && gone(visit));
            tab.history = history;
            tab.position = position;
            changed |= tab.history.len() != before;
        }
        let before = self.closed.len();
        let mut trimmed = false;
        self.closed.retain_mut(|closed| {
            if closed.history.get(closed.position).is_some_and(&gone) {
                return false;
            }
            let count = closed.history.len();
            let (history, position) = without(core::mem::take(&mut closed.history), closed.position, |_, visit| gone(visit));
            closed.history = history;
            closed.position = position;
            trimmed |= closed.history.len() != count;
            true
        });
        changed || trimmed || self.closed.len() != before
    }
}
