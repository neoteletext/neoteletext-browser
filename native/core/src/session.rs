//! The window's tabs, each with its history and its channel. A channel's
//! data is read when it is opened and handed to its decoder at a fixed
//! pace, in the data's own order, never by what is on screen (decision
//! 22): a page arrives whether or not anyone looks at it, and looking at a
//! page never brings it sooner.

use std::collections::BTreeSet;

use interface_core::browser::{StripState, TabInfo};
use interface_core::channel::{BANKS, Channel, PAGES, PAGES_PER_BANK, PageInfo, Status, page_index};
use interface_core::theme::TAB_COLOURS;
use protocol::fetcher::Entry;
use protocol::{MAX_ADDRESS, MAX_BANK_NAME, MAX_IDENTITY, MAX_PAGE_TITLE, MAX_TAB_TITLE, MAX_TITLE, clean};

use crate::address::{Address, parse};
use crate::source::Source;

/// The page a new tab opens, under `.invalid`, which never resolves
/// (RFC 2606 section 2).
pub const HOME: &str = "https://home.invalid/";

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
}

impl ChannelState {
    fn new(id: u32, address: &Address) -> Self {
        let (source, web, problem) = match &address.file {
            Some(path) => match Source::read(path) {
                Ok(source) => (source, None, None),
                Err(problem) => (Source::empty(), None, Some(problem)),
            },
            None => (Source::empty(), Some(address.text.clone()), None),
        };
        Self {
            id,
            title: clean(&address.host, MAX_TITLE),
            banks: vec![String::new(); BANKS],
            titles: vec![None; PAGES],
            loaded: vec![false; PAGES],
            source,
            web,
            problem,
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
                    self.titles[index] = Some(clean(title, MAX_PAGE_TITLE));
                    let bank = index / PAGES_PER_BANK;
                    if self.banks[bank].is_empty() {
                        self.banks[bank].clone_from(&section);
                    }
                }
            }
        }
    }

    /// Records a page the decoder reports; returns its title when the page
    /// is new. A title from the contents file is kept.
    pub fn arrived(&mut self, page: u16, title: &str) -> Option<String> {
        let index = page_index(page)?;
        if self.loaded[index] {
            return None;
        }
        self.loaded[index] = true;
        let title = self.titles[index].get_or_insert_with(|| clean(title, MAX_PAGE_TITLE)).clone();
        Some(title)
    }

    /// The channel as the interface shows it.
    #[must_use]
    pub fn view(&self) -> Channel {
        let mut channel = Channel::empty();
        channel.channel = self.id;
        channel.title.clone_from(&self.title);
        channel.banks.clone_from(&self.banks);
        for (index, title) in self.titles.iter().enumerate() {
            channel.pages[index] =
                title.as_ref().map(|title| PageInfo { title: title.clone(), status: if self.loaded[index] { Status::Loaded } else { Status::Available } });
        }
        channel
    }
}

pub struct Tab {
    pub history: Vec<Address>,
    pub position: usize,
    pub channel: ChannelState,
}

impl Tab {
    #[must_use]
    pub fn address(&self) -> &Address {
        &self.history[self.position]
    }
}

pub struct Session {
    pub tabs: Vec<Tab>,
    pub active: usize,
    bookmarks: BTreeSet<String>,
    next_channel: u32,
}

impl Session {
    #[must_use]
    pub fn new() -> Self {
        Self { tabs: Vec::new(), active: 0, bookmarks: BTreeSet::new(), next_channel: 1 }
    }

    fn channel_for(&mut self, address: &Address) -> ChannelState {
        let id = self.next_channel;
        self.next_channel = self.next_channel.wrapping_add(1).max(1);
        ChannelState::new(id, address)
    }

    /// Opens a tab at `typed` and makes it active.
    ///
    /// # Errors
    /// When the address cannot be opened.
    pub fn open(&mut self, typed: &str) -> Result<&ChannelState, String> {
        let address = parse(typed)?;
        let channel = self.channel_for(&address);
        self.tabs.push(Tab { history: vec![address], position: 0, channel });
        self.active = self.tabs.len() - 1;
        Ok(&self.tabs[self.active].channel)
    }

    /// Opens `typed` in the active tab, after its current address in history.
    ///
    /// # Errors
    /// When the address cannot be opened, or there is no tab.
    pub fn navigate(&mut self, typed: &str) -> Result<(), String> {
        let address = parse(typed)?;
        let channel = self.channel_for(&address);
        let tab = self.tabs.get_mut(self.active).ok_or("no tab is open")?;
        tab.history.truncate(tab.position + 1);
        tab.history.push(address);
        tab.position += 1;
        tab.channel = channel;
        Ok(())
    }

    /// Moves the active tab back or forward in its history. Returns
    /// whether it moved.
    pub fn go(&mut self, forward: bool) -> bool {
        let Some(tab) = self.tabs.get(self.active) else { return false };
        let position = if forward { tab.position + 1 } else { tab.position.wrapping_sub(1) };
        let Some(address) = tab.history.get(position).cloned() else { return false };
        let channel = self.channel_for(&address);
        let tab = &mut self.tabs[self.active];
        tab.position = position;
        tab.channel = channel;
        true
    }

    pub fn toggle_bookmark(&mut self) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let address = tab.address().text.clone();
        if !self.bookmarks.remove(&address) {
            self.bookmarks.insert(address);
        }
    }

    /// Closes tab `index`; returns its channel so its decoder can be closed.
    pub fn close(&mut self, index: usize) -> Option<u32> {
        if index >= self.tabs.len() {
            return None;
        }
        let tab = self.tabs.remove(index);
        if self.active >= self.tabs.len() || index < self.active {
            self.active = self.active.saturating_sub(1);
        }
        Some(tab.channel.id)
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
                let host = &tab.address().host;
                // A tab's colour follows its site, so a site keeps its colour.
                let colour = host.bytes().fold(0_u8, u8::wrapping_add) % u8::try_from(TAB_COLOURS.len()).unwrap_or(1);
                TabInfo { title: clean(host, MAX_TAB_TITLE), colour }
            })
            .collect()
    }

    #[must_use]
    pub fn strip(&self) -> StripState {
        let Some(tab) = self.active_tab() else { return StripState::default() };
        let address = tab.address();
        StripState {
            address: clean(&address.text, MAX_ADDRESS),
            identity: clean(&address.host, MAX_IDENTITY),
            encrypted: address.encrypted,
            local: address.file.is_some(),
            back: tab.position > 0,
            forward: tab.position + 1 < tab.history.len(),
            bookmarked: self.bookmarks.contains(&address.text),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_goes_back_and_forward_and_a_new_address_drops_forward() {
        let mut session = Session::new();
        session.open("news.invalid").unwrap();
        session.navigate("sport.invalid").unwrap();
        assert!(session.strip().back && !session.strip().forward);
        assert!(session.go(false));
        assert_eq!(session.strip().identity, "news.invalid");
        assert!(session.strip().forward);
        session.navigate("weather.invalid").unwrap();
        assert!(!session.strip().forward, "forward history is dropped");
        assert!(!session.go(true));
    }

    #[test]
    fn a_page_is_recorded_once_with_its_title_cleaned() {
        let mut session = Session::new();
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
        let mut session = Session::new();
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
        let mut session = Session::new();
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

    #[test]
    fn bookmarks_follow_the_address() {
        let mut session = Session::new();
        session.open("news.invalid").unwrap();
        session.toggle_bookmark();
        assert!(session.strip().bookmarked);
        session.navigate("sport.invalid").unwrap();
        assert!(!session.strip().bookmarked);
        session.go(false);
        assert!(session.strip().bookmarked);
    }
}
