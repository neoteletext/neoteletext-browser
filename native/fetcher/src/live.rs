//! What a live channel's push connection gives beyond its pages, held
//! until it is whole (neoteletext-app-server's DEC-1467 and DEC-1482,
//! neoteletext-common's DEC-1500): the channel's index, from its index
//! records, as the contents the trusted core takes from `contents.txt`;
//! and each page's list of sub-pages, from its SUBPAGES records, as the
//! catalogue it takes from `NNN.sub`. What is sent is untrusted and
//! checked again in the trusted core.

use std::collections::BTreeMap;

use neoteletext_core::limits::MPP_MAGAZINE_SHIFT;
use protocol::fetcher::{CATALOGUE_PART, Entry, MAX_ENTRIES};
use protocol::subpages::SubPageRows;

/// A page of the index, and the sending it was last listed in.
struct IndexedPage {
    title: String,
    section: String,
    generation: u16,
}

/// A live channel's index: every page its index records listed, the latest
/// listing of each kept (`INPUT.md` section 7.5), and each magazine's name
/// with the sending that gave it.
#[derive(Default)]
pub(crate) struct LiveIndex {
    pages: BTreeMap<u16, IndexedPage>,
    magazines: BTreeMap<u8, (String, u16)>,
}

impl LiveIndex {
    /// A part of a sending: its pages are added, or replace what was
    /// there, and its magazine's name, which a later record of the same
    /// sending leaves empty; nothing is removed until the sending is whole.
    pub(crate) fn note(&mut self, magazine: u8, name: &str, pages: &[(u16, String, String)], generation: u16) {
        if !name.is_empty() {
            self.magazines.insert(magazine, (name.to_owned(), generation));
        }
        for (page, title, section) in pages {
            self.pages.insert(*page, IndexedPage { title: title.clone(), section: section.clone(), generation });
        }
    }

    /// Every part of sending `generation` arrived: a page it did not list
    /// is no longer the channel's.
    pub(crate) fn complete(&mut self, generation: u16) {
        self.pages.retain(|_, page| page.generation == generation);
        self.magazines.retain(|_, (_, given)| *given == generation);
    }

    /// The index as `contents.txt`'s entries: a section wherever a page's
    /// section differs from the page before it, then the page, at most
    /// [`MAX_ENTRIES`] in all. A magazine's name heads its first page, and
    /// is the name of its bank, whatever that page's section.
    pub(crate) fn entries(&self) -> Vec<Entry> {
        let mut entries = Vec::new();
        let mut section = "";
        let mut magazine = None;
        for (&page, indexed) in &self.pages {
            let its = page >> MPP_MAGAZINE_SHIFT;
            if magazine != Some(its) {
                magazine = Some(its);
                if let Some((name, _)) = u8::try_from(its).ok().and_then(|its| self.magazines.get(&its)) {
                    entries.push(Entry::Section(name.clone()));
                    section = &indexed.section;
                }
            }
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

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    fn page(number: u16, section: &str) -> (u16, String, String) {
        (number, format!("Page {number:X}"), section.to_owned())
    }

    fn sections(index: &LiveIndex) -> Vec<String> {
        index.entries().into_iter().filter_map(|entry| if let Entry::Section(name) = entry { Some(name) } else { None }).collect()
    }

    // REQ-1480: a magazine's name heads its pages, kept by a later record
    // of the sending that leaves it empty, and gone with a sending that
    // gives none.
    #[test]
    fn a_magazines_name_heads_its_first_page() {
        let mut index = LiveIndex::default();
        index.note(1, "News", &[page(0x100, "Front")], 1);
        index.note(1, "", &[page(0x101, "Front"), page(0x120, "Sport")], 1);
        index.note(2, "", &[page(0x200, "Other")], 1);
        index.complete(1);
        assert_eq!(sections(&index), ["News", "Sport", "Other"]);
        index.note(1, "", &[page(0x100, "")], 2);
        index.complete(2);
        assert_eq!(sections(&index), [] as [&str; 0]);
    }
}
