//! A tab and its history (`history-and-bookmarks.md` section 2, requirements
//! 20.9 and 20.10): its visits, the visit waiting to be recorded, a closed
//! tab, and the list rules Chrome's are.

use interface_core::library::MAX_TAB_ENTRIES;

use neoteletext::address::Address;

use super::ChannelState;

/// One entry of a tab's history.
#[derive(Clone, Debug)]
pub struct Visit {
    pub address: Address,
    /// A link on a page started it, not the user in the trusted strip
    /// (requirement 8.2).
    pub by_page: bool,
    /// Its title once its page drew, for the Back and Forward lists.
    pub title: String,
    /// When the tab last came to it, in seconds since 1970 (UTC), as
    /// Chrome's `NavigationEntry::GetTimestamp`: clearing history by time
    /// removes the entries of that time (section 2.5).
    pub at: i64,
}

impl Visit {
    pub(super) fn new(address: Address, by_page: bool, at: i64) -> Self {
        Self { address, by_page, title: String::new(), at }
    }

    #[cfg(test)]
    pub(super) fn host_is(&self, host: &str) -> bool {
        self.address.host == host
    }
}

/// A visit waiting to be recorded in history once its page draws
/// (requirement 20.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pending {
    /// Typed in the address field.
    pub typed: bool,
    /// The channel was already drawing: another page of it, or a step back
    /// or forward within it, is recorded at once.
    pub drawn: bool,
    /// A reload, which moves the address's last visit and counts no visit.
    pub reload: bool,
}

/// A tab closed, kept to reopen (requirement 20.9).
#[derive(Clone, Debug)]
pub struct ClosedTab {
    /// Its number, unique in the session.
    pub id: u32,
    /// Its history and where it was in it.
    pub history: Vec<Visit>,
    pub position: usize,
    pub muted: bool,
    /// Its title, for the panel and the menu.
    pub title: String,
    /// The window it closed with, whose closed tabs share this number and
    /// reopen together, as Chrome keeps a closed window; 0 when it closed
    /// alone (`multiple-windows.md`).
    pub window: u32,
}

/// Where going back or forward took the tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Went {
    /// Nowhere: there was no history that way.
    Nowhere,
    /// Another page of the same channel, which stays open.
    Page(Option<u16>),
    /// Another channel.
    Channel,
}

pub struct Tab {
    pub history: Vec<Visit>,
    pub position: usize,
    pub channel: ChannelState,
    /// The user muted the tab; it stays muted as it moves to other channels.
    pub muted: bool,
    /// A private tab, which history never records (requirement 20.8).
    pub private: bool,
    /// The visit to record when its page draws.
    pub pending: Option<Pending>,
}

impl Tab {
    /// Adds a visit after the tab's place, dropping what was forward of it
    /// and, beyond [`MAX_TAB_ENTRIES`], the oldest, as Chrome does
    /// (`NavigationControllerImpl::PruneOldestSkippableEntryIfFull`).
    pub(super) fn push(&mut self, visit: Visit) {
        self.history.truncate(self.position.saturating_add(1));
        self.history.push(visit);
        if self.history.len() > MAX_TAB_ENTRIES {
            self.history.remove(0);
        }
        self.position = self.history.len().saturating_sub(1);
    }

    /// The host of the entry at `position`, which a chapter stop changes.
    fn host_at(&self, position: usize) -> Option<&str> {
        self.history.get(position).map(|visit| visit.address.host.as_str())
    }

    /// The next chapter stop from `start`, as Chrome's
    /// `BackForwardMenuModel::GetIndexOfNextChapterStop`, with the host for
    /// its registrable domain (decision 62 dropped the Public Suffix List):
    /// going back, the nearest earlier entry of another host; going
    /// forward, the last entry of the run of hosts after `start`, the last
    /// entry being always one.
    pub(super) fn next_chapter_stop(&self, start: usize, forward: bool) -> Option<usize> {
        if forward {
            let first = start.checked_add(1).filter(|first| *first < self.history.len())?;
            let host = self.host_at(first)?;
            let end = (first.saturating_add(1)..self.history.len()).find(|&index| self.host_at(index) != Some(host));
            return Some(end.unwrap_or(self.history.len()).saturating_sub(1));
        }
        let host = self.host_at(start)?;
        (0..start).rev().find(|&index| self.host_at(index) != Some(host))
    }

    /// The address the tab is at in its history.
    #[must_use]
    pub fn address(&self) -> Option<&Address> {
        self.history.get(self.position).map(|visit| &visit.address)
    }
}

/// `history` without the entries `gone` names, and `position` moved back by
/// those removed before it.
pub(super) fn without(history: Vec<Visit>, position: usize, gone: impl Fn(usize, &Visit) -> bool) -> (Vec<Visit>, usize) {
    let mut kept = Vec::with_capacity(history.len());
    let mut place = position;
    for (index, visit) in history.into_iter().enumerate() {
        if gone(index, &visit) {
            if index < position {
                place = place.saturating_sub(1);
            }
        } else {
            kept.push(visit);
        }
    }
    let place = place.min(kept.len().saturating_sub(1));
    (kept, place)
}
