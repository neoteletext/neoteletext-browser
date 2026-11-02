//! A channel shown in a tab (`architecture.md` section 6): the pages its
//! contents file lists and the pages its decoder has reported.

use interface_core::browser::{CertificateProblem, Denied};
use interface_core::channel::{BANKS, Channel, PAGES, PAGES_PER_BANK, PageInfo, Status, page_index};
use protocol::fetcher::Entry;
use protocol::{MAX_BANK_NAME, MAX_PAGE_TITLE, MAX_TITLE, clean};

use crate::source::Source;
use neoteletext::address::Address;
use neoteletext::bookmarks::Bookmarks;
use neoteletext::start_page;

use super::Reader;

/// A redirect leaving a channel, while the tab shows where it leads
/// (REQ-1686).
pub struct Redirecting {
    /// Where it leads, by the address rules.
    pub address: Address,
    /// When the notice's second ends.
    pub until: std::time::Instant,
    /// The channels that redirected so far, this one last.
    pub chain: Vec<String>,
    /// Who sent it.
    pub by: RedirectBy,
}

/// What happened to a tab's page since it was selected: what a program's
/// redirect goes by (neoteletext-common's DEC-1684).
#[derive(Clone, Copy, Debug, Default)]
pub struct SinceSelected {
    /// The viewer gave it input (items 6 and 7).
    pub acted: bool,
    /// It drew (item 3).
    pub drawn: bool,
}

/// Who sent a redirect that leaves a channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedirectBy {
    /// A web server, opening the channel (REQ-701).
    Server,
    /// The channel's program (REQ-1688).
    Program,
}

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
    /// Each page's count of sub-pages as its decoder last said, and the
    /// catalogues of sub-pages its fetcher read (decisions 111 and 112).
    pub subpages: neoteletext::subpages::SubPages,
    pub source: Source,
    /// The channel's web address, for its fetcher.
    pub web: Option<String>,
    /// Why the channel has no data, for the log.
    pub problem: Option<String>,
    /// Its certificate was refused, and why, until its contents next arrive
    /// (`certificate-errors.md`, decision 83).
    pub refused: Option<CertificateProblem>,
    /// A restored file not yet read: it is read when its tab is first
    /// shown (finding F-53).
    unread: Option<std::path::PathBuf>,
    /// A live channel: its `channel.ttx` names a push connection, and its
    /// pages and contents come only on it (DEC-1467).
    pub live: bool,
    /// A live channel whose first page has not yet arrived.
    pub connecting: bool,
    /// The channels that redirected here in one navigation, oldest first,
    /// at most [`MAX_REDIRECTS`](super::MAX_REDIRECTS) (REQ-701).
    pub redirects: Vec<String>,
    /// A redirect waiting out its notice. It is the channel's, so anything
    /// that gives the tab another channel drops it (REQ-1686 item 7).
    pub redirecting: Option<Redirecting>,
    /// The page its descriptor opens at, once it was read.
    pub initial_page: Option<u16>,
    /// What happened to the page selected since it was selected.
    pub since_selected: SinceSelected,
    /// The channel refused a selection, and which (page and sub-code), until
    /// a page draws. It stands only while that is what the tab selects.
    pub denied: Option<(Denied, Option<(u16, u16)>)>,
}

impl ChannelState {
    pub(super) fn new(id: u32, address: &Address, bookmarks: &Bookmarks, reader: &Reader) -> Self {
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

    /// A file channel whose file is read only when [`ChannelState::read_now`]
    /// is called, as its tab is first shown (finding F-53).
    pub(super) fn unread(id: u32, address: &Address) -> Self {
        let mut channel = Self::with(id, address, Source::empty(), None, None);
        channel.unread.clone_from(&address.file);
        channel
    }

    /// Reads the file left unread, once; why it has no data, when it
    /// could not be read.
    pub(super) fn read_now(&mut self, reader: &Reader) -> Option<String> {
        let path = self.unread.take()?;
        match reader(&path) {
            Ok(source) => {
                self.source = source;
                None
            }
            Err(problem) => {
                self.problem = Some(problem.clone());
                Some(problem)
            }
        }
    }

    fn with(id: u32, address: &Address, source: Source, web: Option<String>, problem: Option<String>) -> Self {
        Self {
            id,
            title: clean(&address.host, MAX_TITLE),
            banks: vec![String::new(); BANKS],
            titles: vec![None; PAGES],
            loaded: vec![false; PAGES],
            subpages: neoteletext::subpages::SubPages::default(),
            source,
            web,
            problem,
            refused: None,
            unread: None,
            live: false,
            connecting: false,
            redirects: Vec::new(),
            redirecting: None,
            initial_page: None,
            since_selected: SinceSelected::default(),
            denied: None,
        }
    }

    /// Another page is selected: its count of input starts afresh, and it
    /// has not drawn (DEC-1684 item 7).
    pub(super) fn page_selected(&mut self) {
        self.since_selected = SinceSelected::default();
    }

    /// The channel's contents file: every page it lists, with its title,
    /// and the name of the section each hundred falls in.
    pub fn set_contents(&mut self, entries: &[Entry]) {
        // A live channel's index is sent whole each time it changes, so a
        // page it no longer lists goes (DOC-1955).
        if self.live {
            self.titles.fill(None);
            self.banks.fill(String::new());
        }
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
