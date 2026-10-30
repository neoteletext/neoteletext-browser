//! What a live channel's push connection gives beyond its pages, held
//! until it is whole (neoteletext-app-server's DEC-1467 and DEC-1482,
//! neoteletext-common's DEC-1500): the channel's index, from its index
//! records, as the contents the trusted core takes from `contents.txt`;
//! and each page's list of sub-pages, from its SUBPAGES records, as the
//! catalogue it takes from `NNN.sub`. What is sent is untrusted and
//! checked again in the trusted core.

use std::collections::BTreeMap;

use protocol::fetcher::{CATALOGUE_PART, Entry, MAX_ENTRIES};
use protocol::subpages::SubPageRows;

/// A page of the index, and the sending it was last listed in.
struct IndexedPage {
    title: String,
    section: String,
    generation: u16,
}

/// A live channel's index: every page its index records listed, the latest
/// listing of each kept (`INPUT.md` section 7.5).
#[derive(Default)]
pub(crate) struct LiveIndex {
    pages: BTreeMap<u16, IndexedPage>,
}

impl LiveIndex {
    /// A part of a sending: its pages are added, or replace what was
    /// there; nothing is removed until the sending is whole.
    pub(crate) fn note(&mut self, pages: &[(u16, String, String)], generation: u16) {
        for (page, title, section) in pages {
            self.pages.insert(*page, IndexedPage { title: title.clone(), section: section.clone(), generation });
        }
    }

    /// Every part of sending `generation` arrived: a page it did not list
    /// is no longer the channel's.
    pub(crate) fn complete(&mut self, generation: u16) {
        self.pages.retain(|_, page| page.generation == generation);
    }

    /// The index as `contents.txt`'s entries: a section wherever a page's
    /// section differs from the page before it, then the page, at most
    /// [`MAX_ENTRIES`] in all.
    pub(crate) fn entries(&self) -> Vec<Entry> {
        let mut entries = Vec::new();
        let mut section = "";
        for (&page, indexed) in &self.pages {
            if indexed.section != section {
                section = &indexed.section;
                if !section.is_empty() {
                    entries.push(Entry::Section(section.to_owned()));
                }
            }
            entries.push(Entry::Page { page, title: indexed.title.clone() });
        }
        entries.truncate(MAX_ENTRIES);
        entries
    }
}

/// One page's list of sub-pages while its sending arrives.
struct Sending {
    version: u16,
    parts: u16,
    count: u16,
    /// Each part's entries, by part.
    received: BTreeMap<u16, Vec<(u16, String)>>,
}

/// The lists of sub-pages being received, one sending a page.
#[derive(Default)]
pub(crate) struct SubPageLists {
    sending: BTreeMap<u16, Sending>,
}

impl SubPageLists {
    /// Part `part` of `parts` of `page`'s list of `count` sub-pages,
    /// version `version`, from a SUBPAGES record: a record of another
    /// version of the page begins a new list. Returns the page and its
    /// whole list in catalogue parts once every part has arrived; a partial
    /// list gives nothing, so it removes nothing.
    pub(crate) fn note(&mut self, page: u16, version: u16, part: u16, parts: u16, count: u16, entries: Vec<(u16, String)>) -> Option<(u16, Vec<SubPageRows>)> {
        let fresh = || Sending { version, parts, count, received: BTreeMap::new() };
        let sending = self.sending.entry(page).or_insert_with(fresh);
        if (sending.version, sending.parts, sending.count) != (version, parts, count) {
            *sending = fresh();
        }
        sending.received.insert(part, entries);
        if sending.received.len() < usize::from(sending.parts) {
            return None;
        }
        let whole = self.sending.remove(&page)?;
        // The count is the entries the parts carried: the trusted core
        // holds a list once that many have come.
        let entries: Vec<(u16, String)> = whole.received.into_values().flatten().collect();
        let count = u16::try_from(entries.len()).ok()?;
        let parts = entries
            .chunks(CATALOGUE_PART)
            .zip((0_u16..).step_by(CATALOGUE_PART))
            .map(|(rows, first)| SubPageRows { count, first, rows: rows.to_vec() })
            .collect();
        Some((page, parts))
    }
}
