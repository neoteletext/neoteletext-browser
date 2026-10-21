//! The window's tabs, each with its history and its channel. A channel's
//! data is read when it is opened and handed to its decoder at a fixed
//! pace, in the data's own order, never by what is on screen (decision
//! 22): a page arrives whether or not anyone looks at it, and looking at a
//! page never brings it sooner.

use interface_core::browser::{StripState, TabInfo};
use interface_core::channel::{BANKS, Channel, PAGES, PAGES_PER_BANK, PageInfo, Status, page_index};
use interface_core::theme::TAB_COLOURS;
use protocol::fetcher::Entry;
use protocol::{MAX_ADDRESS, MAX_BANK_NAME, MAX_IDENTITY, MAX_PAGE_TITLE, MAX_TAB_TITLE, MAX_TITLE, clean};

use crate::source::Source;
use neoteletext::address::{Address, START_PAGE, parse};
use neoteletext::settings::MAX_BOOKMARKS;
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
}

impl ChannelState {
    fn new(id: u32, address: &Address, bookmarks: &[String]) -> Self {
        // The start page is written here, from the bookmarks, and needs no
        // network (requirement 19.7).
        if address.is_start_page() {
            let mut channel = Self::with(id, address, Source::from_bytes(start_page::pages(bookmarks)), None, None);
            channel.set_contents(&start_page::contents(bookmarks));
            return channel;
        }
        let (source, web, problem) = match &address.file {
            Some(path) => match Source::read(path) {
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
    #[allow(dead_code, reason = "recorded as requirement 8.2 asks; no rule reads it yet, only the tests")]
    pub by_page: bool,
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
}

impl Tab {
    /// The address the tab is at in its history.
    #[must_use]
    pub fn address(&self) -> Option<&Address> {
        self.history.get(self.position).map(|visit| &visit.address)
    }
}

pub struct Session {
    pub tabs: Vec<Tab>,
    pub active: usize,
    /// In the order added, at most [`MAX_BOOKMARKS`] (requirement 19.5).
    bookmarks: Vec<String>,
    next_channel: u32,
}

impl Session {
    #[must_use]
    pub fn new(bookmarks: Vec<String>) -> Self {
        Self { tabs: Vec::new(), active: 0, bookmarks, next_channel: 1 }
    }

    fn channel_for(&mut self, address: &Address) -> ChannelState {
        let id = self.next_channel;
        self.next_channel = self.next_channel.wrapping_add(1).max(1);
        ChannelState::new(id, address, &self.bookmarks)
    }

    /// The bookmarks, in the order added.
    #[must_use]
    pub fn bookmarks(&self) -> &[String] {
        &self.bookmarks
    }

    /// The bookmark the active tab's start page opens at `page`, if the
    /// tab shows the start page (requirement 19.7).
    #[must_use]
    pub fn start_page_bookmark(&self, channel: u32, page: u16) -> Option<String> {
        let tab = self.active_tab().filter(|tab| tab.channel.id == channel)?;
        tab.address().filter(|address| address.is_start_page())?;
        start_page::bookmark_at(page, &self.bookmarks).map(str::to_owned)
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
        self.visit(address, false)
    }

    /// Opens a tab at `typed` and makes it active.
    ///
    /// # Errors
    /// When the address cannot be opened.
    pub fn open(&mut self, typed: &str) -> Result<&ChannelState, String> {
        let address = parse(typed)?;
        let channel = self.channel_for(&address);
        self.active = self.tabs.len();
        self.tabs.push(Tab { history: vec![Visit { address, by_page: false }], position: 0, channel, muted: false });
        self.tabs.last().map(|tab| &tab.channel).ok_or_else(|| "no tab is open".into())
    }

    /// Opens `typed` in the active tab, after its current address in history.
    ///
    /// # Errors
    /// When the address cannot be opened, or there is no tab.
    pub fn navigate(&mut self, typed: &str) -> Result<(), String> {
        let address = parse(typed)?;
        self.visit(address, false)
    }

    /// Opens another channel in the active tab because a link on a page led
    /// there and the user confirmed it in the trusted strip.
    ///
    /// # Errors
    /// When there is no tab.
    #[cfg(feature = "links")]
    pub fn follow(&mut self, address: Address) -> Result<(), String> {
        self.visit(address, true)
    }

    fn visit(&mut self, address: Address, by_page: bool) -> Result<(), String> {
        let channel = self.channel_for(&address);
        let tab = self.tabs.get_mut(self.active).ok_or("no tab is open")?;
        tab.history.truncate(tab.position.saturating_add(1));
        tab.history.push(Visit { address, by_page });
        tab.position = tab.history.len().saturating_sub(1);
        tab.channel = channel;
        Ok(())
    }

    /// A link on a page of the active tab led to another page of its
    /// channel: the tab's history gains the page, and the channel stays open.
    /// Returns the address the strip now shows.
    #[cfg(feature = "links")]
    pub fn show_page(&mut self, page: u16) -> Option<&Address> {
        let tab = self.tabs.get_mut(self.active)?;
        let address = tab.address()?.with_page(page);
        tab.history.truncate(tab.position.saturating_add(1));
        tab.history.push(Visit { address, by_page: true });
        tab.position = tab.history.len().saturating_sub(1);
        tab.address()
    }

    /// Moves the active tab back or forward in its history. Another page of
    /// the same channel keeps the channel open.
    pub fn go(&mut self, forward: bool) -> Went {
        let Some(tab) = self.tabs.get(self.active) else { return Went::Nowhere };
        let position = if forward { tab.position.checked_add(1) } else { tab.position.checked_sub(1) };
        let Some(position) = position else { return Went::Nowhere };
        let Some(visit) = tab.history.get(position).cloned() else { return Went::Nowhere };
        let same = tab.address().is_some_and(|current| current.same_channel(&visit.address));
        let channel = (!same).then(|| self.channel_for(&visit.address));
        let Some(tab) = self.tabs.get_mut(self.active) else { return Went::Nowhere };
        tab.position = position;
        match channel {
            Some(channel) => {
                tab.channel = channel;
                Went::Channel
            }
            None => Went::Page(visit.address.page),
        }
    }

    /// Bookmarks the active tab's channel, or removes its bookmark; returns
    /// whether the bookmarks changed. A channel is kept by its address
    /// without a page, and at most [`MAX_BOOKMARKS`] are kept.
    pub fn toggle_bookmark(&mut self) -> bool {
        let Some(address) = self.tabs.get(self.active).and_then(Tab::address) else { return false };
        let address = address.channel_text().to_owned();
        if let Some(index) = self.bookmarks.iter().position(|bookmark| *bookmark == address) {
            self.bookmarks.remove(index);
            return true;
        }
        if self.bookmarks.len() >= MAX_BOOKMARKS {
            return false;
        }
        self.bookmarks.push(address);
        true
    }

    /// Mutes tab `index`, or unmutes it. Returns whether there is such a tab.
    pub fn toggle_mute(&mut self, index: usize) -> bool {
        let Some(tab) = self.tabs.get_mut(index) else { return false };
        tab.muted = !tab.muted;
        true
    }

    /// Mutes every tab, or unmutes every tab.
    pub fn mute_all(&mut self, muted: bool) {
        for tab in &mut self.tabs {
            tab.muted = muted;
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
                let host = tab.address().map_or("", |address| address.host.as_str());
                // A tab's colour follows its site, so a site keeps its colour.
                let colour = host.bytes().fold(0_u8, u8::wrapping_add).checked_rem(u8::try_from(TAB_COLOURS.len()).unwrap_or(1)).unwrap_or(0);
                TabInfo { title: clean(host, MAX_TAB_TITLE), colour, channel: tab.channel.id, muted: tab.muted }
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
            bookmarked: self.bookmarks.iter().any(|bookmark| bookmark == address.channel_text()),
            built_in: address.is_start_page(),
            notice: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tab_stays_muted_on_its_next_channel_and_mute_all_reaches_every_tab() {
        let mut session = Session::new(Vec::new());
        session.open("news.invalid").unwrap();
        session.open("sport.invalid").unwrap();
        assert!(session.toggle_mute(0));
        assert!(!session.toggle_mute(2), "no such tab");
        let first = session.tab_infos()[0].channel;
        session.active = 0;
        session.navigate("weather.invalid").unwrap();
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
        let mut session = Session::new(Vec::new());
        session.open("news.invalid").unwrap();
        session.navigate("sport.invalid").unwrap();
        assert!(session.strip().back && !session.strip().forward);
        assert_eq!(session.go(false), Went::Channel);
        assert_eq!(session.strip().identity, "news.invalid");
        assert!(session.strip().forward);
        session.navigate("weather.invalid").unwrap();
        assert!(!session.strip().forward, "forward history is dropped");
        assert_eq!(session.go(true), Went::Nowhere);
    }

    #[cfg(feature = "links")]
    #[test]
    fn a_link_within_the_channel_keeps_it_open_and_is_recorded_as_the_pages() {
        let mut session = Session::new(Vec::new());
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
        let mut session = Session::new(Vec::new());
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
        let mut session = Session::new(Vec::new());
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
        let mut session = Session::new(Vec::new());
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
        let mut session = Session::new(Vec::new());
        session.open("news.invalid").unwrap();
        session.toggle_bookmark();
        assert!(session.strip().bookmarked);
        session.navigate("sport.invalid").unwrap();
        assert!(!session.strip().bookmarked);
        assert_eq!(session.go(false), Went::Channel);
        assert!(session.strip().bookmarked);
        assert_eq!(session.bookmarks(), ["https://news.invalid/"]);
    }

    /// Requirement 19.5: a channel once, by its address without a page, in
    /// the order added, at most 1 000.
    #[test]
    fn bookmarks_are_kept_in_order_and_bounded() {
        let full: Vec<String> = (0..MAX_BOOKMARKS).map(|n| format!("https://b{n}.invalid/")).collect();
        let mut session = Session::new(full);
        session.open("news.invalid/#302").unwrap();
        assert!(!session.toggle_bookmark(), "no room");
        assert!(!session.strip().bookmarked);
        session.navigate("b5.invalid").unwrap();
        assert!(session.strip().bookmarked);
        assert!(session.toggle_bookmark(), "removed");
        assert_eq!(session.bookmarks().len(), MAX_BOOKMARKS - 1);
        session.navigate("news.invalid/#302").unwrap();
        assert!(session.toggle_bookmark());
        assert_eq!(session.bookmarks().last().map(String::as_str), Some("https://news.invalid/"), "added last, without its page");
    }

    /// Requirement 19.7: the start page is served from the bookmarks with
    /// no fetcher, and its bookmark pages open them as the user's own
    /// navigation; back returns to its front page.
    #[test]
    fn the_start_page_opens_its_bookmarks() {
        let mut session = Session::new(vec!["https://news.invalid/".into()]);
        session.open(START_PAGE).unwrap();
        let channel = &session.tabs[0].channel;
        assert!(channel.web.is_none() && channel.problem.is_none(), "no network");
        assert_eq!(channel.view().pages[start_page_index(0x201)].as_ref().map(|page| page.title.as_str()), Some("news.invalid/"));
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
        session.navigate("news.invalid").unwrap();
        let id = session.tabs[0].channel.id;
        assert_eq!(session.start_page_bookmark(id, 0x201), None, "only on the start page");
    }

    fn start_page_index(page: u16) -> usize {
        page_index(page).unwrap()
    }
}
