//! The window's tabs, each with its history and its channel. A channel's
//! data is read when it is opened and handed to its decoder at a fixed
//! pace, in the data's own order, never by what is on screen (decision
//! 22): a page arrives whether or not anyone looks at it, and looking at a
//! page never brings it sooner.

use std::collections::VecDeque;
use std::path::Path;

use interface_core::browser::{CertificateProblem, StripState, TabInfo};
use interface_core::channel::{BANKS, Channel, PAGES, PAGES_PER_BANK, PageInfo, Status, page_index};
use interface_core::theme::TAB_COLOURS;
use protocol::fetcher::Entry;
use protocol::{MAX_ADDRESS, MAX_BANK_NAME, MAX_IDENTITY, MAX_PAGE_TITLE, MAX_TAB_TITLE, MAX_TITLE, clean};

use crate::source::Source;
use interface_core::library::{MAX_CHAPTER_STOPS, MAX_CLOSED, MAX_TAB_ENTRIES, MAX_TAB_LIST, MAX_WINDOWS, WindowGeometry};
use neoteletext::address::{Address, START_PAGE, parse};
use neoteletext::bookmarks::Bookmarks;
use neoteletext::restore::{self, Saved};
use neoteletext::start_page;

/// One channel shown in a tab: the pages its contents file lists and the
/// pages its decoder has reported.
pub struct ChannelState {
    pub id: u32,
    pub title: String,
    /// Section names by hundred, from the contents file.
    pub banks: Vec<String>,
    /// Each page's title, by page index: from the contents file, or else
    /// from the page itself when it arrives.
    pub titles: Vec<Option<String>>,
    /// The pages that have arrived.
    pub loaded: Vec<bool>,
    pub source: Source,
    /// The channel's web address, for its fetcher.
    pub web: Option<String>,
    /// Why the channel has no data, for the log.
    pub problem: Option<String>,
    /// Its certificate was refused, and why, until its contents next arrive
    /// (`certificate-errors.md`, decision 83).
    pub refused: Option<CertificateProblem>,
}

impl ChannelState {
    fn new(id: u32, address: &Address, bookmarks: &Bookmarks, reader: &Reader) -> Self {
        // The start page is written here, from the bookmarks, and needs no
        // network (requirement 19.7).
        if address.is_start_page() {
            let listed = bookmarks.listed();
            let mut channel = Self::with(id, address, Source::from_bytes(start_page::pages(&listed)), None, None);
            channel.set_contents(&start_page::contents(&listed));
            return channel;
        }
        let (source, web, problem) = match &address.file {
            Some(path) => match reader(path) {
                Ok(source) => (source, None, None),
                Err(problem) => (Source::empty(), None, Some(problem)),
            },
            // A web channel is fetched without its fragment, which names a
            // page and is never sent (`hyperlinks.md` section 7.3).
            None => (Source::empty(), Some(address.channel_text().to_owned()), None),
        };
        Self::with(id, address, source, web, problem)
    }

    fn with(id: u32, address: &Address, source: Source, web: Option<String>, problem: Option<String>) -> Self {
        Self {
            id,
            title: clean(&address.host, MAX_TITLE),
            banks: vec![String::new(); BANKS],
            titles: vec![None; PAGES],
            loaded: vec![false; PAGES],
            source,
            web,
            problem,
            refused: None,
        }
    }

    /// The channel's contents file: every page it lists, with its title,
    /// and the name of the section each hundred falls in.
    pub fn set_contents(&mut self, entries: &[Entry]) {
        let mut section = String::new();
        for entry in entries {
            match entry {
                Entry::Section(name) => section = clean(name, MAX_BANK_NAME),
                Entry::Page { page, title } => {
                    let Some(index) = page_index(*page) else { continue };
                    if let Some(slot) = self.titles.get_mut(index) {
                        *slot = Some(clean(title, MAX_PAGE_TITLE));
                    }
                    if let Some(bank) = self.banks.get_mut(index / PAGES_PER_BANK)
                        && bank.is_empty()
                    {
                        bank.clone_from(&section);
                    }
                }
            }
        }
    }

    /// Whether the channel has `page`: listed in its contents, or arrived.
    #[cfg(feature = "links")]
    #[must_use]
    pub fn has_page(&self, page: u16) -> bool {
        page_index(page).is_some_and(|index| self.titles.get(index).is_some_and(Option::is_some) || self.loaded.get(index).is_some_and(|loaded| *loaded))
    }

    /// A page's title, from the contents file or the page itself.
    #[must_use]
    pub fn page_title(&self, page: u16) -> Option<&str> {
        page_index(page).and_then(|index| self.titles.get(index)).and_then(Option::as_deref)
    }

    /// Records a page the decoder reports; returns its title when the page
    /// is new. A title from the contents file is kept.
    pub fn arrived(&mut self, page: u16, title: &str) -> Option<String> {
        let index = page_index(page)?;
        let loaded = self.loaded.get_mut(index)?;
        if *loaded {
            return None;
        }
        *loaded = true;
        let title = self.titles.get_mut(index)?.get_or_insert_with(|| clean(title, MAX_PAGE_TITLE)).clone();
        Some(title)
    }

    /// The channel as the interface shows it.
    #[must_use]
    pub fn view(&self) -> Channel {
        let mut channel = Channel::empty();
        channel.channel = self.id;
        channel.title.clone_from(&self.title);
        channel.banks.clone_from(&self.banks);
        for ((slot, title), &loaded) in channel.pages.iter_mut().zip(&self.titles).zip(&self.loaded) {
            *slot = title.as_ref().map(|title| PageInfo { title: title.clone(), status: if loaded { Status::Loaded } else { Status::Available } });
        }
        channel
    }
}

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
    fn new(address: Address, by_page: bool, at: i64) -> Self {
        Self { address, by_page, title: String::new(), at }
    }

    #[cfg(test)]
    fn host_is(&self, host: &str) -> bool {
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

/// A window set aside while another is at hand (`multiple-windows.md`):
/// its tabs, which is active, its place, and whether it is private.
pub struct Window {
    pub id: u32,
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub geometry: Option<WindowGeometry>,
    pub private: bool,
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
    fn push(&mut self, visit: Visit) {
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
    fn next_chapter_stop(&self, start: usize, forward: bool) -> Option<usize> {
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
    /// Tabs closed, the most recent first, at most [`MAX_CLOSED`]
    /// (requirement 20.9).
    pub closed: VecDeque<ClosedTab>,
    next_closed: u32,
    next_channel: u32,
    reader: Reader,
    /// The time, in seconds since 1970 (UTC), that the trusted core set
    /// before the change at hand: what a tab's entry is stamped with.
    pub now: i64,
}

/// `history` without the entries `gone` names, and `position` moved back by
/// those removed before it.
fn without(history: Vec<Visit>, position: usize, gone: impl Fn(usize, &Visit) -> bool) -> (Vec<Visit>, usize) {
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
    /// A session that reads file channels from the disk itself, for tests.
    #[cfg(test)]
    #[must_use]
    pub fn new(bookmarks: Bookmarks) -> Self {
        Self::with_reader(bookmarks, Box::new(|path: &Path| Source::read(path)))
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

    /// Opens `typed` in the active tab, after its current address in
    /// history; `in_field` when the user typed it in the address field.
    ///
    /// # Errors
    /// When the address cannot be opened, or there is no tab.
    pub fn navigate(&mut self, typed: &str, in_field: bool) -> Result<(), String> {
        let address = parse(typed)?;
        // The address shown, entered again, is a reload, as in Chrome
        // (`chrome.history`'s "reload" transition: "pressing Enter in the
        // address bar").
        if self.is_shown(&address) {
            self.reload();
            return Ok(());
        }
        self.visit(address, false, in_field)
    }

    /// Whether `address` is the one the active tab shows.
    #[must_use]
    pub fn is_shown(&self, address: &Address) -> bool {
        self.active_tab().and_then(Tab::address).is_some_and(|shown| shown.text == address.text)
    }

    /// Opens another channel in the active tab because a link on a page led
    /// there and the user confirmed it in the trusted strip.
    ///
    /// # Errors
    /// When there is no tab.
    #[cfg(feature = "links")]
    pub fn follow(&mut self, address: Address) -> Result<(), String> {
        // A link to the address shown is a reload, as in Chrome
        // (`ShouldTreatNavigationAsReload`).
        if self.is_shown(&address) {
            self.reload();
            return Ok(());
        }
        self.visit(address, true, false)
    }

    fn visit(&mut self, address: Address, by_page: bool, typed: bool) -> Result<(), String> {
        let channel = self.channel_for(&address);
        let now = self.now;
        let tab = self.tabs.get_mut(self.active).ok_or("no tab is open")?;
        tab.push(Visit::new(address, by_page, now));
        tab.channel = channel;
        tab.pending = Some(Pending { typed, drawn: false, reload: false });
        Ok(())
    }

    /// A link on a page of the active tab led to another page of its
    /// channel: the tab's history gains the page, and the channel stays open.
    /// Returns the address the strip now shows.
    #[cfg(feature = "links")]
    pub fn show_page(&mut self, page: u16) -> Option<&Address> {
        let tab = self.tabs.get_mut(self.active)?;
        let address = tab.address()?.with_page(page);
        // A link to the page shown is a reload, which the caller makes.
        if tab.address().is_some_and(|shown| shown.text == address.text) {
            return None;
        }
        tab.push(Visit::new(address, true, self.now));
        tab.pending = Some(Pending { typed: false, drawn: true, reload: false });
        tab.address()
    }

    /// Opens the active tab's channel afresh where the tab is in its
    /// history: a new decoder and fetcher, its pages read again (requirement
    /// 11.2). The history is as it was. False when no tab is open.
    pub fn reload(&mut self) -> bool {
        let Some(address) = self.active_tab().and_then(Tab::address).cloned() else { return false };
        let channel = self.channel_for(&address);
        let now = self.now;
        let Some(tab) = self.tabs.get_mut(self.active) else { return false };
        tab.channel = channel;
        if let Some(visit) = tab.history.get_mut(tab.position) {
            visit.at = now;
        }
        tab.pending = Some(Pending { typed: false, drawn: false, reload: true });
        true
    }

    /// Moves the active tab back or forward in its history. Another page of
    /// the same channel keeps the channel open.
    pub fn go(&mut self, forward: bool) -> Went {
        self.go_to(forward, 1)
    }

    /// Moves the active tab `steps` back or forward in its history, as
    /// Back or Forward pressed that many times (requirement 20.10).
    pub fn go_to(&mut self, forward: bool, steps: usize) -> Went {
        let Some(tab) = self.tabs.get(self.active) else { return Went::Nowhere };
        let position = if forward { tab.position.checked_add(steps) } else { tab.position.checked_sub(steps) };
        let Some(position) = position.filter(|_| steps > 0) else { return Went::Nowhere };
        let Some(visit) = tab.history.get(position).cloned() else { return Went::Nowhere };
        let same = tab.address().is_some_and(|current| current.same_channel(&visit.address));
        let channel = (!same).then(|| self.channel_for(&visit.address));
        let now = self.now;
        let Some(tab) = self.tabs.get_mut(self.active) else { return Went::Nowhere };
        tab.position = position;
        if let Some(visit) = tab.history.get_mut(position) {
            visit.at = now;
        }
        tab.pending = Some(Pending { typed: false, drawn: same, reload: false });
        match channel {
            Some(channel) => {
                tab.channel = channel;
                Went::Channel
            }
            None => Went::Page(visit.address.page),
        }
    }

    /// The active tab's entries one way, the nearest first, at most
    /// [`MAX_TAB_LIST`] (requirement 20.10).
    #[must_use]
    pub fn tab_list(&self, forward: bool) -> Vec<(usize, &Visit)> {
        let Some(tab) = self.active_tab() else { return Vec::new() };
        (1..=MAX_TAB_LIST)
            .map_while(|steps| {
                let position = if forward { tab.position.checked_add(steps) } else { tab.position.checked_sub(steps) }?;
                tab.history.get(position).map(|visit| (steps, visit))
            })
            .collect()
    }

    /// After a full list, at most [`MAX_CHAPTER_STOPS`] entries beyond it
    /// where the host changes, with their steps, as Chrome's
    /// `BackForwardMenuModel::GetChapterStopCount` and `FindChapterStop`.
    #[must_use]
    pub fn chapter_stops(&self, forward: bool) -> Vec<(usize, &Visit)> {
        let Some(tab) = self.active_tab() else { return Vec::new() };
        if self.tab_list(forward).len() < MAX_TAB_LIST {
            return Vec::new();
        }
        let start = if forward { tab.position.checked_add(MAX_TAB_LIST) } else { tab.position.checked_sub(MAX_TAB_LIST) };
        let mut stops = Vec::new();
        let mut at = start;
        while stops.len() < MAX_CHAPTER_STOPS {
            let Some(stop) = at.and_then(|at| tab.next_chapter_stop(at, forward)) else { break };
            if let Some(visit) = tab.history.get(stop) {
                stops.push((stop.abs_diff(tab.position), visit));
            }
            at = Some(stop);
        }
        stops
    }

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

    /// Keeps a closed tab to reopen, unless it is private.
    fn keep_closed(&mut self, tab: Tab, window: u32) {
        if tab.private {
            return;
        }
        let title = clean(tab.address().map_or("", |address| address.host.as_str()), MAX_TAB_TITLE);
        let id = self.next_closed;
        self.next_closed = self.next_closed.wrapping_add(1).max(1);
        self.closed.push_front(ClosedTab { id, history: tab.history, position: tab.position, muted: tab.muted, title, window });
        self.closed.truncate(MAX_CLOSED);
    }

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
                let channel = self.channel_for(&address);
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
mod tests {
    use super::*;

    #[test]
    fn a_tab_stays_muted_on_its_next_channel_and_mute_all_reaches_every_tab() {
        let mut session = Session::new(Bookmarks::default());
        session.open("news.invalid").unwrap();
        session.open("sport.invalid").unwrap();
        assert!(session.toggle_mute(0));
        assert!(!session.toggle_mute(2), "no such tab");
        let first = session.tab_infos()[0].channel;
        session.active = 0;
        session.navigate("weather.invalid", true).unwrap();
        let infos = session.tab_infos();
        assert!(infos[0].channel != first && infos[0].muted, "the tab's new channel is muted");
        assert!(!infos[1].muted);
        session.mute_all(true);
        assert!(session.tab_infos().iter().all(|tab| tab.muted));
        session.mute_all(false);
        assert!(session.tab_infos().iter().all(|tab| !tab.muted));
    }

    #[test]
    fn history_goes_back_and_forward_and_a_new_address_drops_forward() {
        let mut session = Session::new(Bookmarks::default());
        session.open("news.invalid").unwrap();
        session.navigate("sport.invalid", true).unwrap();
        assert!(session.strip().back && !session.strip().forward);
        assert_eq!(session.go(false), Went::Channel);
        assert_eq!(session.strip().identity, "news.invalid");
        assert!(session.strip().forward);
        session.navigate("weather.invalid", true).unwrap();
        assert!(!session.strip().forward, "forward history is dropped");
        assert_eq!(session.go(true), Went::Nowhere);
    }

    #[cfg(feature = "links")]
    #[test]
    fn a_link_within_the_channel_keeps_it_open_and_is_recorded_as_the_pages() {
        let mut session = Session::new(Bookmarks::default());
        session.open("news.invalid/teletext/").unwrap();
        let channel = session.tabs[0].channel.id;
        assert_eq!(session.show_page(0x302).map(|address| address.text.as_str()), Some("https://news.invalid/teletext/#302"));
        assert!(session.tabs[0].history[1].by_page && !session.tabs[0].history[0].by_page);
        assert_eq!(session.tabs[0].channel.id, channel, "the same channel, still open");
        assert_eq!(session.go(false), Went::Page(None));
        assert_eq!(session.go(true), Went::Page(Some(0x302)));
        assert_eq!(session.tabs[0].channel.id, channel);
        // A link to another channel opens it, recorded as the page's.
        session.follow(parse("https://sport.invalid/#401").unwrap()).unwrap();
        assert!(session.tabs[0].history[2].by_page);
        assert_ne!(session.tabs[0].channel.id, channel);
        assert_eq!(session.tabs[0].channel.web.as_deref(), Some("https://sport.invalid/"), "the fragment is never fetched");
    }

    #[test]
    fn a_page_is_recorded_once_with_its_title_cleaned() {
        let mut session = Session::new(Bookmarks::default());
        session.open("news.invalid").unwrap();
        let channel = &mut session.tabs[0].channel;
        assert!(channel.web.is_some(), "a web channel has a fetcher");
        assert_eq!(channel.arrived(0x100, "Head\u{202E}lines").as_deref(), Some("Headlines"));
        assert_eq!(channel.arrived(0x100, "Again"), None);
        assert_eq!(channel.arrived(0x1A0, "Hexadecimal"), None);
        assert_eq!(channel.view().counts(), (1, 1));
    }

    #[test]
    fn the_contents_file_lists_pages_and_names_sections() {
        let mut session = Session::new(Bookmarks::default());
        session.open("news.invalid").unwrap();
        let channel = &mut session.tabs[0].channel;
        channel.set_contents(&[
            Entry::Section("News".into()),
            Entry::Page { page: 0x100, title: "Front page".into() },
            Entry::Page { page: 0x101, title: "Headlines".into() },
            Entry::Section("Sport".into()),
            Entry::Page { page: 0x300, title: "Sport index".into() },
        ]);
        let view = channel.view();
        assert_eq!(view.counts(), (3, 0), "listed, none arrived yet");
        assert_eq!((view.banks[0].as_str(), view.banks[2].as_str()), ("News", "Sport"));
        // The page's own title does not replace the contents file's.
        assert_eq!(channel.arrived(0x101, "Something else").as_deref(), Some("Headlines"));
        assert_eq!(channel.view().counts(), (3, 1));
    }

    #[test]
    fn closing_tabs_keeps_a_valid_active_tab() {
        let mut session = Session::new(Bookmarks::default());
        for host in ["a.invalid", "b.invalid", "c.invalid"] {
            session.open(host).unwrap();
        }
        assert_eq!(session.active, 2);
        session.close(2);
        assert_eq!(session.active, 1);
        session.close(0);
        assert_eq!(session.active, 0);
        assert_eq!(session.strip().identity, "b.invalid");
    }

    /// Requirement 20.16: the star is filled for the address the strip
    /// shows, page and all.
    #[test]
    fn bookmarks_follow_the_address() {
        let mut session = Session::new(Bookmarks::default());
        session.open("news.invalid/#302").unwrap();
        assert!(!session.strip().bookmarked);
        session.bookmarks.add(neoteletext::bookmarks::OTHER, "", "news.invalid/#302").unwrap();
        assert!(session.strip().bookmarked);
        session.navigate("news.invalid", true).unwrap();
        assert!(!session.strip().bookmarked, "the channel without the page is another address");
        assert_eq!(session.go(false), Went::Page(Some(0x302)), "the same channel");
        assert!(session.strip().bookmarked);
    }

    /// Requirement 20.9: a closed tab comes back where it was, with its
    /// whole history; a private one is not kept; at most 25.
    #[test]
    fn closed_tabs_reopen_where_they_were() {
        let mut session = Session::new(Bookmarks::default());
        session.open("news.invalid").unwrap();
        session.navigate("sport.invalid", true).unwrap();
        session.open_private("secret.invalid").unwrap();
        session.close(1);
        session.close(0);
        assert_eq!(session.closed.len(), 1, "the private tab is not kept");
        assert_eq!(session.closed[0].title, "sport.invalid");
        session.open("weather.invalid").unwrap();
        session.reopen(None).unwrap();
        assert_eq!(session.strip().identity, "sport.invalid");
        assert!(session.strip().back, "its history came back with it");
        assert_eq!(session.go(false), Went::Channel);
        assert_eq!(session.strip().identity, "news.invalid");
        assert!(session.reopen(None).is_err());
        for n in 0..30 {
            session.open(&format!("b{n}.invalid")).unwrap();
            session.close(session.active);
        }
        assert_eq!(session.closed.len(), MAX_CLOSED);
        let id = session.closed[3].id;
        session.reopen(Some(id)).unwrap();
        assert_eq!(session.strip().identity, "b26.invalid");
    }

    /// Requirements 20.21 and 20.22: the session kept is every tab but the
    /// private ones, in order, each with its list, place and mute; restored,
    /// it is the same, with the same tab active, each tab waiting for its
    /// page to draw.
    #[test]
    fn the_session_is_kept_and_restored() {
        let mut session = Session::new(Bookmarks::default());
        session.open("news.invalid").unwrap();
        session.navigate("sport.invalid", true).unwrap();
        session.go(false);
        session.open_private("secret.invalid").unwrap();
        session.open("weather.invalid").unwrap();
        session.toggle_mute(2);
        session.tabs[0].history[0].title = "NEWS".into();
        let window = Some(WindowGeometry { x: 5, y: 6, width: 700, height: 500, maximised: true });
        session.geometry = window;
        let saved = session.saved();
        let kept = &saved.windows[0];
        assert_eq!(kept.geometry, window);
        let lists: Vec<Vec<&str>> = kept.tabs.iter().map(|tab| tab.entries.iter().map(|entry| entry.address.as_str()).collect()).collect();
        assert_eq!(lists, [vec!["https://news.invalid/", "https://sport.invalid/"], vec!["https://weather.invalid/"]]);
        assert_eq!((kept.tabs[0].position, kept.tabs[0].entries[0].title.as_str()), (0, "NEWS"));
        assert_eq!((kept.active, kept.tabs[1].muted), (1, true));

        let mut restored = Session::new(Bookmarks::default());
        assert_eq!(restored.restore(&saved), 2);
        assert_eq!(restored.saved(), saved, "restored as it was");
        assert_eq!(restored.active, 1);
        assert!(restored.tabs.iter().all(|tab| tab.pending == Some(Pending { typed: false, drawn: false, reload: false })));
        restored.active = 0;
        assert_eq!(restored.go(true), Went::Channel, "its forward list came back with it");
        assert_eq!(restored.strip().identity, "sport.invalid");

        session.active = 1;
        assert_eq!(session.saved().windows[0].active, 0, "a private active tab keeps the one before it");
        session.close(2);
        session.close(0);
        assert!(session.saved().windows.is_empty(), "only a private tab: none kept");
    }

    /// Requirement 20.10: the lists one way, the nearest first, and going
    /// several steps at once.
    #[test]
    fn back_and_forward_lists_go_several_steps() {
        let mut session = Session::new(Bookmarks::default());
        session.open("a.invalid").unwrap();
        for host in ["b.invalid", "c.invalid", "d.invalid"] {
            session.navigate(host, true).unwrap();
        }
        let back: Vec<&str> = session.tab_list(false).iter().map(|(_, visit)| visit.address.host.as_str()).collect();
        assert_eq!(back, ["c.invalid", "b.invalid", "a.invalid"]);
        assert!(session.tab_list(true).is_empty());
        assert_eq!(session.go_to(false, 3), Went::Channel);
        assert_eq!(session.strip().identity, "a.invalid");
        assert_eq!(session.go_to(false, 1), Went::Nowhere);
        assert_eq!(session.go_to(true, 0), Went::Nowhere);
        assert_eq!(session.tab_list(true).len(), 3);
    }

    /// Requirement 20.1: each navigation waits to be recorded; typed in the
    /// field or not; within a channel, at once.
    #[test]
    fn navigations_wait_to_be_recorded() {
        let mut session = Session::new(Bookmarks::default());
        session.open("news.invalid/teletext/").unwrap();
        assert_eq!(session.tabs[0].pending, Some(Pending { typed: false, drawn: false, reload: false }));
        session.tabs[0].pending = None;
        session.navigate("sport.invalid", true).unwrap();
        assert_eq!(session.tabs[0].pending, Some(Pending { typed: true, drawn: false, reload: false }));
        session.navigate("sport.invalid/#401", false).unwrap();
        session.tabs[0].pending = None;
        assert_eq!(session.go(false), Went::Page(None));
        assert_eq!(session.tabs[0].pending, Some(Pending { typed: false, drawn: true, reload: false }), "the same channel, already drawing");
        session.open_private("news.invalid").unwrap();
        assert!(session.tab_infos()[1].private && session.strip().private);
    }

    /// Requirement 20.10, as Chrome's Back list: twelve entries, then,
    /// beyond them, up to five chapter stops where the host changes; a
    /// tab keeps at most 50 entries, the oldest dropped.
    #[test]
    fn back_lists_twelve_then_chapter_stops_and_a_tab_keeps_fifty() {
        let mut session = Session::new(Bookmarks::default());
        session.open("a.invalid").unwrap();
        // a, then b×3, c×3, d×3, e×3, f×3, g×3, h×3: 22 entries, the last h.
        for host in ["b", "c", "d", "e", "f", "g", "h"] {
            for page in ["#100", "#200", "#300"] {
                session.navigate(&format!("{host}.invalid/{page}"), false).unwrap();
            }
        }
        let hosts = |list: Vec<(usize, &Visit)>| list.into_iter().map(|(steps, visit)| (steps, visit.address.host.clone())).collect::<Vec<_>>();
        assert_eq!(session.tab_list(false).len(), MAX_TAB_LIST);
        // The twelfth back is at position 21 - 12 = 9, a d; the stops are
        // the nearest earlier entry of another host each time: c at 6,
        // b at 3, a at 0.
        assert_eq!(hosts(session.chapter_stops(false)), [(15, "c.invalid".into()), (18, "b.invalid".into()), (21, "a.invalid".into())]);
        assert!(session.chapter_stops(true).is_empty(), "nothing forward");
        assert_eq!(session.go_to(false, 21), Went::Channel);
        // Forward from a: twelve, then the last entry of each run of hosts
        // after the twelfth (position 12, an e): f ends at 15, g at 18, and
        // the last entry, an h, at 21.
        assert_eq!(hosts(session.chapter_stops(true)), [(15, "f.invalid".into()), (18, "g.invalid".into()), (21, "h.invalid".into())]);
        for page in 0..60 {
            session.navigate(&format!("z.invalid/#{:03}", 100 + page), false).unwrap();
        }
        let tab = &session.tabs[0];
        assert_eq!((tab.history.len(), tab.position), (MAX_TAB_ENTRIES, MAX_TAB_ENTRIES - 1), "the oldest dropped");
    }

    /// Requirement 20.6, as Chrome: removing history removes those entries
    /// from the tabs' lists but never the entry a tab is at, drops a closed
    /// tab whose entry it was at, and leaves private tabs alone.
    #[test]
    fn removed_history_leaves_the_tabs_lists() {
        let mut session = Session::new(Bookmarks::default());
        let addresses = |history: &[Visit]| history.iter().map(|visit| visit.address.host.clone()).collect::<Vec<_>>();
        for (at, host) in [(100, "a"), (200, "b"), (300, "c")] {
            session.now = at;
            if session.tabs.is_empty() {
                session.open(&format!("{host}.invalid")).unwrap();
            } else {
                session.navigate(&format!("{host}.invalid"), false).unwrap();
            }
        }
        session.now = 400;
        session.open_private("p.invalid").unwrap();
        session.navigate("q.invalid", false).unwrap();
        // A closed tab at b, and one at c.
        for host in ["b", "c"] {
            session.now = 500;
            session.open(&format!("{host}.invalid")).unwrap();
            session.close(session.tabs.len() - 1);
        }
        assert_eq!(session.closed.len(), 2);
        session.active = 0;
        assert_eq!(session.go_to(false, 1), Went::Channel, "the first tab at b");
        // Everything from 150 on: the first tab keeps a and b, which it is
        // at; the closed tabs were at b and c from 500, so both go.
        assert!(session.remove_entries(|visit| visit.at >= 150 && visit.address.host != "a.invalid"));
        assert_eq!(addresses(&session.tabs[0].history), ["a.invalid", "b.invalid"]);
        assert_eq!(session.tabs[0].position, 1);
        assert_eq!(addresses(&session.tabs[1].history), ["p.invalid", "q.invalid"], "a private tab is not touched");
        assert!(session.closed.is_empty());
        assert!(!session.remove_entries(|visit| visit.host_is("z.invalid")), "nothing named, nothing changed");
    }

    /// Requirement 20.9, as Chrome: the recently closed tabs, with each
    /// entry's time, are kept with the session and come back with it.
    #[test]
    fn closed_tabs_are_kept_between_runs() {
        let mut session = Session::new(Bookmarks::default());
        session.now = 1_000;
        session.open("a.invalid").unwrap();
        session.open("b.invalid").unwrap();
        session.navigate("c.invalid", false).unwrap();
        session.close(1);
        let saved = session.saved();
        assert_eq!(saved.closed.len(), 1);
        let mut restored = Session::new(Bookmarks::default());
        restored.restore(&Saved::read(&saved.text()).0);
        let closed = &restored.closed[0];
        assert_eq!((closed.position, closed.title.as_str()), (1, "c.invalid"));
        assert_eq!(closed.history.iter().map(|visit| visit.at).collect::<Vec<_>>(), [1_000, 1_000]);
        restored.reopen(None).unwrap();
        assert_eq!(restored.active_tab().and_then(Tab::address).map(|address| address.host.as_str()), Some("c.invalid"));
    }

    /// Decision 69: a tab moves; the active tab stays active wherever it
    /// goes and wherever the others go round it.
    #[test]
    fn tabs_move_and_the_active_one_stays_active() {
        let mut session = Session::new(Bookmarks::default());
        for host in ["a", "b", "c", "d"] {
            session.open(&format!("{host}.invalid")).unwrap();
        }
        let hosts = |session: &Session| session.tabs.iter().map(|tab| tab.address().unwrap().host.clone()).collect::<Vec<_>>();
        session.active = 1;
        assert!(session.move_tab(3, 0));
        assert_eq!(hosts(&session), ["d.invalid", "a.invalid", "b.invalid", "c.invalid"]);
        assert_eq!(session.active, 2, "b moved right with the others");
        assert!(session.move_tab(2, 3));
        assert_eq!((hosts(&session)[3].as_str(), session.active), ("b.invalid", 3), "the active tab itself");
        assert!(session.move_tab(0, 2));
        assert_eq!(session.active, 3, "a tab passing from before it to before it: unchanged");
        assert!(!session.move_tab(1, 1) && !session.move_tab(0, 9), "nowhere to go");
    }

    /// Requirement 11.6: a reload gives the tab a new channel at the same
    /// address, its history as it was, and waits to record a reload.
    #[test]
    fn a_reload_opens_the_channel_afresh_where_the_tab_is() {
        let mut session = Session::new(Bookmarks::default());
        session.open("news.invalid/teletext/").unwrap();
        session.navigate("sport.invalid/#401", false).unwrap();
        assert_eq!(session.go(false), Went::Channel);
        let addresses = |tab: &Tab| tab.history.iter().map(|visit| visit.address.text.clone()).collect::<Vec<_>>();
        let (before, history, position) = (session.tabs[0].channel.id, addresses(&session.tabs[0]), session.tabs[0].position);
        session.tabs[0].pending = None;
        assert!(session.reload());
        let tab = &session.tabs[0];
        assert_ne!(tab.channel.id, before, "a new channel: a new decoder and fetcher");
        assert_eq!((addresses(tab), tab.position), (history.clone(), position), "the history as it was");
        assert_eq!(tab.address().map(|address| address.text.clone()), history.get(position).cloned());
        assert_eq!(tab.pending, Some(Pending { typed: false, drawn: false, reload: true }));
        assert!(!Session::new(Bookmarks::default()).reload(), "no tab, nothing to reload");
        // The address shown, entered again or linked to, is a reload, as in
        // Chrome: no new entry.
        let shown = session.tabs[0].address().map(|address| address.text.clone()).unwrap();
        session.tabs[0].pending = None;
        session.navigate(&shown, true).unwrap();
        assert_eq!((addresses(&session.tabs[0]), session.tabs[0].position), (history.clone(), position));
        assert_eq!(session.tabs[0].pending, Some(Pending { typed: false, drawn: false, reload: true }), "not typed, a reload");
        #[cfg(feature = "links")]
        {
            session.follow(parse(&shown).unwrap()).unwrap();
            assert_eq!(addresses(&session.tabs[0]), history, "a link to it is one too");
        }
    }

    /// Requirement 19.7: the start page is served from the bookmarks with
    /// no fetcher, and its bookmark pages open them as the user's own
    /// navigation; back returns to its front page.
    #[test]
    fn the_start_page_opens_its_bookmarks() {
        let mut bookmarks = Bookmarks::default();
        bookmarks.add(neoteletext::bookmarks::BAR, "News", "https://news.invalid/").unwrap();
        let mut session = Session::new(bookmarks);
        session.open(START_PAGE).unwrap();
        let channel = &session.tabs[0].channel;
        assert!(channel.web.is_none() && channel.problem.is_none(), "no network");
        assert_eq!(channel.view().pages[start_page_index(0x201)].as_ref().map(|page| page.title.as_str()), Some("News"));
        assert!(session.strip().built_in);
        let id = channel.id;
        assert_eq!(session.start_page_bookmark(id, 0x201).as_deref(), Some("https://news.invalid/"));
        assert_eq!(session.start_page_bookmark(id, 0x202), None);
        assert_eq!(session.start_page_bookmark(id.wrapping_add(1), 0x201), None, "another channel's page");
        session.tabs[0].history[0].address = parse("neoteletext:home#201").unwrap();
        session.open_from_start_page("https://news.invalid/").unwrap();
        assert_eq!(session.strip().identity, "news.invalid");
        assert!(!session.tabs[0].history[1].by_page, "the user's own navigation");
        assert_eq!(session.go(false), Went::Channel);
        assert_eq!(session.strip().address, START_PAGE, "back at the front page, not the bookmark's");
        let id = session.tabs[0].channel.id;
        assert_eq!(session.start_page_bookmark(id, 0x201).as_deref(), Some("https://news.invalid/"));
        session.navigate("news.invalid", true).unwrap();
        let id = session.tabs[0].channel.id;
        assert_eq!(session.start_page_bookmark(id, 0x201), None, "only on the start page");
    }

    /// `multiple-windows.md`: each window keeps its own tabs and active
    /// tab; switching sets one aside and brings the other back.
    #[test]
    fn windows_keep_their_own_tabs() {
        let mut session = Session::new(Bookmarks::default());
        session.open("a.invalid").unwrap();
        session.open("b.invalid").unwrap();
        session.active = 0;
        let first = session.window;
        let second = session.new_window(false).unwrap();
        assert!(session.tabs.is_empty() && session.window == second && session.window_count() == 2);
        session.open("c.invalid").unwrap();
        let c = session.tabs[0].channel.id;
        assert_eq!(session.window_of(c), Some(second));
        assert!(session.switch_to(first));
        assert_eq!((session.tabs.len(), session.active), (2, 0), "the first window as it was");
        assert_eq!(session.window_of(c), Some(second));
        assert_eq!(session.all_tabs().count(), 3);
        assert!(!session.switch_to(99), "no such window");
        assert_eq!(session.window_ids(), [first, second]);
        // A private window's tabs are private.
        let private = session.new_window(true).unwrap();
        session.open("d.invalid").unwrap();
        assert!(session.tabs[0].private);
        session.switch_to(first);
        session.open("e.invalid").unwrap();
        assert!(!session.tabs[2].private, "only in the private window");
        assert_eq!(session.window_ids(), [first, second, private]);
    }

    /// A tab moves between windows; a window closes with its tabs, which
    /// reopen together in a window of their own, as Chrome reopens a closed
    /// window.
    #[test]
    fn a_closed_window_reopens_whole() {
        let mut session = Session::new(Bookmarks::default());
        session.open("a.invalid").unwrap();
        let first = session.window;
        let second = session.new_window(false).unwrap();
        session.open("b.invalid").unwrap();
        session.open("c.invalid").unwrap();
        let moved = session.take_tab(1).unwrap();
        session.switch_to(first);
        session.put_tab(moved, 0);
        assert_eq!(session.active_tab().and_then(Tab::address).map(|address| address.host.as_str()), Some("c.invalid"));
        session.switch_to(second);
        session.open("d.invalid").unwrap();
        assert!(session.close_window().len() == 2 && session.window == first && session.window_count() == 1);
        let groups: Vec<(&str, bool)> = session.closed.iter().map(|closed| (closed.title.as_str(), closed.window == second)).collect();
        assert_eq!(groups, [("d.invalid", true), ("b.invalid", true)]);
        assert!(session.close_window().is_empty(), "the last window does not close");
        session.reopen(None).unwrap();
        assert_eq!(session.window_count(), 2);
        let hosts: Vec<&str> = session.tabs.iter().map(|tab| tab.address().unwrap().host.as_str()).collect();
        assert_eq!(hosts, ["b.invalid", "d.invalid"], "in their order, in a new window");
        assert!(session.closed.is_empty());
    }

    /// Every window is kept with the session, and restored as windows.
    #[test]
    fn every_window_is_kept_and_restored() {
        let mut session = Session::new(Bookmarks::default());
        session.open("a.invalid").unwrap();
        session.geometry = Some(WindowGeometry { x: 0, y: 0, width: 800, height: 600, maximised: false });
        session.new_window(false).unwrap();
        session.open("b.invalid").unwrap();
        session.open("c.invalid").unwrap();
        session.active = 0;
        session.geometry = Some(WindowGeometry { x: 900, y: 0, width: 700, height: 500, maximised: false });
        session.new_window(true).unwrap();
        session.open("secret.invalid").unwrap();
        let saved = session.saved();
        assert_eq!(saved.windows.len(), 2, "a private window is not kept");
        let mut restored = Session::new(Bookmarks::default());
        assert_eq!(restored.restore(&Saved::read(&saved.text()).0), 3);
        assert_eq!(restored.window_count(), 2);
        assert_eq!(restored.saved(), saved);
        assert_eq!(restored.tabs.len(), 1, "the first window is the one at hand");
    }

    /// The session holds at most `MAX_TABS` tabs in all its windows.
    #[test]
    fn tabs_are_bounded_across_windows() {
        let mut session = Session::new(Bookmarks::default());
        for _ in 0..MAX_TABS / 2 {
            session.open("a.invalid").unwrap();
        }
        session.new_window(false).unwrap();
        for _ in 0..MAX_TABS / 2 {
            session.open("b.invalid").unwrap();
        }
        assert!(session.open("c.invalid").is_err());
        assert!(session.room_for_a_tab().is_err());
        assert_eq!(session.tab_count(), MAX_TABS);
    }

    fn start_page_index(page: u16) -> usize {
        page_index(page).unwrap()
    }
}
