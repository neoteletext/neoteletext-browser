//! A channel shown in a tab (`architecture.md` section 6): the pages its
//! contents file lists and the pages its decoder has reported.

use interface_core::browser::CertificateProblem;
use interface_core::channel::{BANKS, Channel, PAGES, PAGES_PER_BANK, PageInfo, Status, page_index};
use protocol::fetcher::Entry;
use protocol::{MAX_BANK_NAME, MAX_PAGE_TITLE, MAX_TITLE, clean};

use crate::source::Source;
use neoteletext::address::Address;
use neoteletext::bookmarks::Bookmarks;
use neoteletext::start_page;

use super::Reader;

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
    /// The sub-pages of each page that has more than one, by page number,
    /// each sub-code with its cleaned title, as its decoder last said.
    pub subpages: std::collections::BTreeMap<u16, Vec<(u16, String)>>,
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
            subpages: std::collections::BTreeMap::new(),
            source,
            web,
            problem,
            refused: None,
            unread: None,
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

    /// Records `page`'s sub-pages as its decoder reports them, their titles
    /// cleaned; returns them when they changed, an empty list when the page
    /// has one sub-page again.
    pub fn set_subpages(&mut self, page: u16, titles: &[(u16, String)]) -> Option<Vec<(u16, String)>> {
        page_index(page)?;
        let cleaned: Vec<(u16, String)> =
            titles.iter().take(interface_core::controls::MAX_SUBPAGES).map(|(subcode, title)| (*subcode, clean(title, MAX_PAGE_TITLE))).collect();
        let old = if cleaned.is_empty() { self.subpages.remove(&page) } else { self.subpages.insert(page, cleaned.clone()) };
        (old.as_ref() != Some(&cleaned) && !(old.is_none() && cleaned.is_empty())).then_some(cleaned)
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
