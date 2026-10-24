//! The window's tabs, each with its history and its channel. A channel's
//! data is read when it is opened and handed to its decoder at a fixed
//! pace, in the data's own order, never by what is on screen (decision
//! 22): a page arrives whether or not anyone looks at it, and looking at a
//! page never brings it sooner.

use std::collections::VecDeque;
use std::path::Path;

use interface_core::browser::{StripState, TabInfo};
use interface_core::library::WindowGeometry;
use interface_core::theme::TAB_COLOURS;
use protocol::{MAX_ADDRESS, MAX_IDENTITY, MAX_TAB_TITLE, clean};

use crate::source::Source;
use neoteletext::address::{Address, START_PAGE, parse};
use neoteletext::bookmarks::Bookmarks;
use neoteletext::start_page;

mod channel;
mod closed;
#[cfg(feature = "links")]
mod links;
mod navigation;
mod saved;
mod tab;
mod windows;

pub use channel::ChannelState;
pub use tab::{ClosedTab, Pending, Tab, Visit, Went};
pub use windows::Window;

/// The first window's number, as the interface numbers it; numbers are
/// never `protocol::NEW_WINDOW`.
pub const FIRST_WINDOW: u32 = protocol::FIRST_WINDOW;
/// The most tabs open, in all windows: what one window's strip may list,
/// and what the session file keeps.
pub const MAX_TABS: usize = protocol::MAX_TABS;

/// How a file channel's bytes are read: by the broker in the browser
/// (decision 56), from the disk in tests.
pub type Reader = Box<dyn Fn(&Path) -> Result<Source, String>>;

pub struct Session {
    /// The tabs of the window at hand, and which is active.
    pub tabs: Vec<Tab>,
    pub active: usize,
    /// The window at hand: the one whose tabs `tabs` are, which the
    /// interface's last intent came from or the event at hand is for.
    pub window: u32,
    /// Its place and size, as the interface last reported them.
    pub geometry: Option<WindowGeometry>,
    /// It is a private window: every tab opened in it is private.
    pub private_window: bool,
    /// The other windows, set aside.
    pub others: Vec<Window>,
    next_window: u32,
    /// The bookmarks (requirement 20.15).
    pub bookmarks: Bookmarks,
    /// Tabs closed, the most recent first, at most
    /// [`MAX_CLOSED`](interface_core::library::MAX_CLOSED)
    /// (requirement 20.9).
    pub closed: VecDeque<ClosedTab>,
    next_closed: u32,
    next_channel: u32,
    reader: Reader,
    /// The time, in seconds since 1970 (UTC), that the trusted core set
    /// before the change at hand: what a tab's entry is stamped with.
    pub now: i64,
}

impl Session {
    /// A session that reads file channels from the disk itself, for tests.
    #[cfg(test)]
    #[must_use]
    pub fn new(bookmarks: Bookmarks) -> Self {
        Self::with_reader(bookmarks, Box::new(|path: &Path| Source::read(path, &crate::source::Places::new(None, Vec::new())).map_err(String::from)))
    }

    /// A session that reads file channels with `reader`.
    #[must_use]
    pub fn with_reader(bookmarks: Bookmarks, reader: Reader) -> Self {
        Self {
            tabs: Vec::new(),
            active: 0,
            window: FIRST_WINDOW,
            geometry: None,
            private_window: false,
            others: Vec::new(),
            next_window: FIRST_WINDOW.saturating_add(1),
            bookmarks,
            closed: VecDeque::new(),
            next_closed: 1,
            next_channel: 1,
            reader,
            now: 0,
        }
    }

    fn channel_for(&mut self, address: &Address) -> ChannelState {
        let id = self.next_channel;
        self.next_channel = self.next_channel.wrapping_add(1).max(1);
        ChannelState::new(id, address, &self.bookmarks, &self.reader)
    }

    /// A channel for a restored tab: a file is read only when its tab is
    /// first shown ([`Session::read_unread`]), not every restored file
    /// through the broker at start (finding F-53).
    fn channel_later(&mut self, address: &Address) -> ChannelState {
        if address.file.is_none() {
            return self.channel_for(address);
        }
        let id = self.next_channel;
        self.next_channel = self.next_channel.wrapping_add(1).max(1);
        ChannelState::unread(id, address)
    }

    /// Reads the file of `channel`'s tab if it was left unread at start;
    /// why it has no data, when it could not be read.
    pub fn read_unread(&mut self, channel: u32) -> Option<String> {
        let reader = &self.reader;
        let tab = self.tabs.iter_mut().chain(self.others.iter_mut().flat_map(|window| window.tabs.iter_mut())).find(|tab| tab.channel.id == channel)?;
        tab.channel.read_now(reader)
    }

    /// The bookmark the active tab's start page opens at `page`, if the
    /// tab shows the start page (requirement 19.7).
    #[must_use]
    pub fn start_page_bookmark(&self, channel: u32, page: u16) -> Option<String> {
        let tab = self.active_tab().filter(|tab| tab.channel.id == channel)?;
        tab.address().filter(|address| address.is_start_page())?;
        start_page::bookmark_at(page, &self.bookmarks.listed())
    }

    /// Opens a bookmark the start page chose, as the user's own navigation:
    /// the start page's place in history becomes its front page, so going
    /// back returns to it rather than opening the bookmark again.
    ///
    /// # Errors
    /// When the address cannot be opened, or there is no tab.
    #[cfg_attr(not(feature = "links"), allow(dead_code, reason = "only a confirmed link opens a bookmark from the start page"))]
    pub fn open_from_start_page(&mut self, bookmark: &str) -> Result<(), String> {
        let address = parse(bookmark)?;
        let front = parse(START_PAGE)?;
        let tab = self.tabs.get_mut(self.active).ok_or("no tab is open")?;
        if let Some(visit) = tab.history.get_mut(tab.position).filter(|visit| visit.address.is_start_page()) {
            visit.address = front;
        }
        self.visit(address, false, false)
    }

    /// Opens a tab at `typed` and makes it active.
    ///
    /// # Errors
    /// When the address cannot be opened.
    pub fn open(&mut self, typed: &str) -> Result<&ChannelState, String> {
        self.open_tab(typed, false)
    }

    /// Opens a private tab at `typed` and makes it active (requirement 20.8).
    ///
    /// # Errors
    /// When the address cannot be opened.
    pub fn open_private(&mut self, typed: &str) -> Result<&ChannelState, String> {
        self.open_tab(typed, true)
    }

    fn open_tab(&mut self, typed: &str, private: bool) -> Result<&ChannelState, String> {
        self.room_for_a_tab()?;
        let private = private || self.private_window;
        let address = parse(typed)?;
        let channel = self.channel_for(&address);
        self.active = self.tabs.len();
        let pending = Some(Pending { typed: false, drawn: false, reload: false });
        self.tabs.push(Tab { history: vec![Visit::new(address, false, self.now)], position: 0, channel, muted: false, private, pending });
        self.tabs.last().map(|tab| &tab.channel).ok_or_else(|| "no tab is open".into())
    }

    /// Whether another tab may open: the session holds at most
    /// [`MAX_TABS`] in all its windows.
    ///
    /// # Errors
    /// When it is full.
    pub fn room_for_a_tab(&self) -> Result<(), String> {
        if self.tab_count() >= MAX_TABS { Err(format!("At most {MAX_TABS} tabs are open at once")) } else { Ok(()) }
    }

    /// How many tabs are open, in every window.
    #[must_use]
    pub fn tab_count(&self) -> usize {
        self.others.iter().map(|window| window.tabs.len()).sum::<usize>().saturating_add(self.tabs.len())
    }

    /// Mutes tab `index`, or unmutes it. Returns whether there is such a tab.
    pub fn toggle_mute(&mut self, index: usize) -> bool {
        let Some(tab) = self.tabs.get_mut(index) else { return false };
        tab.muted = !tab.muted;
        true
    }

    /// Mutes every tab of every window, or unmutes every tab.
    pub fn mute_all(&mut self, muted: bool) {
        for tab in self.all_tabs_mut() {
            tab.muted = muted;
        }
    }

    /// Closes tab `index`, keeping it to reopen unless it is private;
    /// returns its channel so its decoder can be closed.
    /// Moves the tab at `from` to `to`, the others closing up, the active
    /// tab staying active (`tab-strip.md` section 2, decision 69). Returns
    /// whether anything moved.
    pub fn move_tab(&mut self, from: usize, to: usize) -> bool {
        if from == to || from >= self.tabs.len() || to >= self.tabs.len() {
            return false;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        self.active = if self.active == from {
            to
        } else if from < self.active && self.active <= to {
            self.active.saturating_sub(1)
        } else if to <= self.active && self.active < from {
            self.active.saturating_add(1)
        } else {
            self.active
        };
        true
    }

    pub fn close(&mut self, index: usize) -> Option<u32> {
        if index >= self.tabs.len() {
            return None;
        }
        let tab = self.take_tab(index)?;
        let channel = tab.channel.id;
        self.keep_closed(tab, 0);
        Some(channel)
    }

    #[must_use]
    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    #[must_use]
    pub fn tab_infos(&self) -> Vec<TabInfo> {
        self.tabs
            .iter()
            .map(|tab| {
                let host = tab.address().map_or("", |address| address.host.as_str());
                // A tab's colour follows its site, so a site keeps its colour.
                let colour = host.bytes().fold(0_u8, u8::wrapping_add).checked_rem(u8::try_from(TAB_COLOURS.len()).unwrap_or(1)).unwrap_or(0);
                TabInfo { title: clean(host, MAX_TAB_TITLE), colour, channel: tab.channel.id, muted: tab.muted, private: tab.private }
            })
            .collect()
    }

    #[must_use]
    pub fn strip(&self) -> StripState {
        let Some(tab) = self.active_tab() else { return StripState::default() };
        let Some(address) = tab.address() else { return StripState::default() };
        StripState {
            address: clean(&address.text, MAX_ADDRESS),
            identity: clean(&address.host, MAX_IDENTITY),
            encrypted: address.encrypted,
            local: address.file.is_some(),
            back: tab.position > 0,
            forward: tab.position.saturating_add(1) < tab.history.len(),
            bookmarked: self.bookmarks.find(&address.text).is_some(),
            built_in: address.is_start_page(),
            private: tab.private,
            notice: String::new(),
            refused: tab.channel.refused,
        }
    }
}

#[cfg(test)]
mod tests;
