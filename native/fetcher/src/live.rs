//! What a live channel's push connection gives beyond its pages, held
//! until it is whole (neoteletext-app-server's DEC-1467 and DEC-1482,
//! neoteletext-common's DEC-1500): the channel's index, from its index
//! records, as the contents the trusted core takes from `contents.txt`;
//! and each page's list of sub-pages, from its SUBPAGES records, in the
//! parts it takes from `NNN/P.sub`. What is sent is untrusted and checked
//! again in the trusted core.

use std::collections::BTreeMap;

use neoteletext_core::limits::MPP_MAGAZINE_SHIFT;
use protocol::fetcher::{Entry, MAX_ENTRIES, ROWS_PER_PART};
use protocol::subpages::SubPageRows;

/// A page of the index, and the sending it was last listed in.
struct IndexedPage {
    title: String,
    section: String,
    generation: u16,
}

/// A live channel's index: every page its index records listed, the latest
/// listing of each kept (DOC-1955), and each magazine's name
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

/// What came of a SUBPAGES record.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Noted {
    /// Its sending is not whole yet: nothing changes.
    Partial,
    /// The page's whole list, its version as the parts of a list carry
    /// it, and its rows in the parts of `NNN/P.sub`.
    Whole(u16, String, Vec<SubPageRows>),
    /// The page's sending, whole, breaks its totals: it is discarded whole
    /// and the page keeps the list it had (DOC-1952, DOC-1959).
    Refused(u16, Refusal),
}

/// Why a whole sending was discarded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Its parts carried another number of sub-pages than its count.
    Count,
    /// A sub-code is not above the one before it, across its parts: out of
    /// order, or given twice.
    Order,
}

impl SubPageLists {
    /// Part `part` of `parts` of `page`'s list of `count` sub-pages,
    /// version `version`, from a SUBPAGES record: a record of another
    /// version of the page begins a new list. Once every part has arrived
    /// the list is whole when its entries number `count` and its sub-codes
    /// ascend over all its parts, each once, and is discarded otherwise; a
    /// partial list gives nothing, so it removes nothing. Each record is
    /// checked alone by the engine's reader; only the whole sending can
    /// break these totals.
    pub(crate) fn note(&mut self, page: u16, version: u16, part: u16, parts: u16, count: u16, entries: Vec<(u16, String)>) -> Noted {
        let fresh = || Sending { version, parts, count, received: BTreeMap::new() };
        let sending = self.sending.entry(page).or_insert_with(fresh);
        if (sending.version, sending.parts, sending.count) != (version, parts, count) {
            *sending = fresh();
        }
        sending.received.insert(part, entries);
        if sending.received.len() < usize::from(sending.parts) {
            return Noted::Partial;
        }
        let Some(whole) = self.sending.remove(&page) else { return Noted::Partial };
        let entries: Vec<(u16, String)> = whole.received.into_values().flatten().collect();
        if entries.len() != usize::from(whole.count) {
            return Noted::Refused(page, Refusal::Count);
        }
        if entries.windows(2).any(|pair| matches!(pair, [(before, _), (after, _)] if after <= before)) {
            return Noted::Refused(page, Refusal::Order);
        }
        let parts = entries
            .chunks(ROWS_PER_PART)
            .zip((0_u16..).step_by(ROWS_PER_PART))
            .map(|(rows, first)| SubPageRows { count: whole.count, first, rows: rows.to_vec() })
            .collect();
        Noted::Whole(page, whole.version.to_string(), parts)
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

    fn rows(subcodes: &[u16]) -> Vec<(u16, String)> {
        subcodes.iter().map(|&subcode| (subcode, format!("Story {subcode:04X}"))).collect()
    }

    // DOC-1959, DOC-1952: a sending whose parts, each valid alone, repeat a
    // sub-code or go back across a part, or carry another number of
    // sub-pages than its count, is discarded whole, and the list the page
    // had stands. On the code before, each was taken as the page's list.
    #[test]
    fn a_sending_that_breaks_its_totals_is_discarded() {
        let mut lists = SubPageLists::default();
        assert_eq!(lists.note(0x210, 1, 0, 2, 4, rows(&[1, 2])), Noted::Partial);
        assert_eq!(lists.note(0x210, 1, 1, 2, 4, rows(&[2, 3])), Noted::Refused(0x210, Refusal::Order), "repeated across parts");
        lists.note(0x210, 2, 0, 2, 4, rows(&[3, 4]));
        assert_eq!(lists.note(0x210, 2, 1, 2, 4, rows(&[1, 2])), Noted::Refused(0x210, Refusal::Order), "back across parts");
        lists.note(0x210, 3, 0, 2, 5, rows(&[1, 2]));
        assert_eq!(lists.note(0x210, 3, 1, 2, 5, rows(&[3, 4])), Noted::Refused(0x210, Refusal::Count), "four of five");
        lists.note(0x210, 4, 0, 2, 4, rows(&[1, 2]));
        let Noted::Whole(0x210, version, parts) = lists.note(0x210, 4, 1, 2, 4, rows(&[3, 4])) else { panic!("a sound sending is whole") };
        assert_eq!(version, "4");
        assert_eq!(parts, [SubPageRows { count: 4, first: 0, rows: rows(&[1, 2, 3, 4]) }]);
    }

    fn pages(index: &LiveIndex) -> Vec<u16> {
        index.entries().into_iter().filter_map(|entry| if let Entry::Page { page, .. } = entry { Some(page) } else { None }).collect()
    }

    // DOC-1955, TODO-1531 item 5: each part of a sending shows its pages as it
    // arrives; a whole sending removes the pages it did not list, and a
    // partial one removes nothing.
    #[test]
    fn a_sending_in_parts_shows_each_part_and_a_whole_one_removes_the_rest() {
        let mut index = LiveIndex::default();
        index.note(1, "News", &[page(0x100, "Front"), page(0x101, "Front")], 1);
        assert_eq!(pages(&index), [0x100, 0x101], "the first part, at once");
        index.note(2, "Sport", &[page(0x200, "Football")], 1);
        assert_eq!(pages(&index), [0x100, 0x101, 0x200], "both parts");
        index.complete(1);
        assert_eq!(pages(&index), [0x100, 0x101, 0x200]);
        index.note(1, "News", &[page(0x100, "Front")], 2);
        assert_eq!(pages(&index), [0x100, 0x101, 0x200], "a partial sending removes nothing");
        index.note(3, "Weather", &[page(0x300, "Today")], 2);
        index.complete(2);
        assert_eq!(pages(&index), [0x100, 0x300], "a whole sending removes the pages it did not list");
        assert_eq!(sections(&index), ["News", "Weather"]);
    }

    // DOC-1955, TODO-1531 item 5: the index never gives the trusted core more
    // than the contents file's MAX_ENTRIES entries, however a channel lists it.
    #[test]
    fn the_index_holds_at_most_max_entries() {
        let mut index = LiveIndex::default();
        // Every page of the eight magazines, each in a section of its own:
        // 800 pages and 800 sections.
        let every: Vec<_> = (1..=8_u16)
            .flat_map(|magazine| (0..100_u16).map(move |number| (magazine << 8) | ((number / 10) << 4) | (number % 10)))
            .map(|number| page(number, &format!("Section {number:X}")))
            .collect();
        index.note(1, "", &every, 1);
        index.complete(1);
        let entries = index.entries();
        assert_eq!(entries.len(), MAX_ENTRIES);
        assert_eq!(entries.first(), Some(&Entry::Section("Section 100".to_owned())));
        assert_eq!(entries.last(), Some(&Entry::Page { page: 0x599, title: "Page 599".to_owned() }), "the first thousand, in order");
    }

    // REQ-1505, REQ-1503, TODO-1531 item 6: the largest list, 8,190 sub-pages
    // in many records, reaches the trusted core whole, in ascending order, in
    // parts of ROWS_PER_PART rows each at its first place; and only when its
    // last record has arrived.
    #[test]
    fn the_largest_list_arrives_whole_in_parts_of_the_trusted_cores_size() {
        let count = neoteletext_core::limits::MAX_SUBPAGES;
        let all: Vec<u16> = (1..=u16::try_from(count).unwrap()).collect();
        // The records as a channel might send them: 100 sub-pages each.
        let records: Vec<&[u16]> = all.chunks(100).collect();
        let parts = u16::try_from(records.len()).unwrap();
        let count = u16::try_from(count).unwrap();
        let mut lists = SubPageLists::default();
        let mut noted = Vec::new();
        for (part, subcodes) in records.iter().enumerate() {
            noted.push(lists.note(0x602, 7, u16::try_from(part).unwrap(), parts, count, rows(subcodes)));
        }
        let Some(Noted::Whole(0x602, version, whole)) = noted.pop() else { panic!("whole at the last record") };
        assert!(noted.iter().all(|noted| *noted == Noted::Partial), "nothing before it");
        assert_eq!(version, "7");
        assert_eq!(whole.len(), all.len().div_ceil(ROWS_PER_PART));
        for (index, part) in whole.iter().enumerate() {
            let first = index * ROWS_PER_PART;
            let expected = &all[first..(first + ROWS_PER_PART).min(all.len())];
            assert_eq!((part.count, usize::from(part.first)), (count, first), "part {index}");
            assert_eq!(part.rows, rows(expected), "part {index}");
        }
    }

    // DEC-1526, TODO-1531 item 6: 0000 beside other sub-codes is the page's
    // own content and an ordinary first entry.
    #[test]
    fn sub_code_0000_beside_others_stays_first() {
        let mut lists = SubPageLists::default();
        lists.note(0x210, 1, 0, 2, 3, rows(&[0x0000]));
        let Noted::Whole(_, _, parts) = lists.note(0x210, 1, 1, 2, 3, rows(&[0x0001, 0x0002])) else { panic!("whole") };
        assert_eq!(parts, [SubPageRows { count: 3, first: 0, rows: rows(&[0x0000, 0x0001, 0x0002]) }]);
    }

    // DOC-1959, TODO-1531 item 6: a record of a new version begins a new
    // sending, so the old version's parts are never joined to it, and a
    // partial sending gives nothing.
    #[test]
    fn a_new_version_discards_the_old_parts() {
        let mut lists = SubPageLists::default();
        assert_eq!(lists.note(0x210, 1, 0, 2, 4, rows(&[1, 2])), Noted::Partial);
        assert_eq!(lists.note(0x210, 2, 1, 2, 4, rows(&[7, 8])), Noted::Partial, "version 1's first part is not version 2's");
        let Noted::Whole(_, version, parts) = lists.note(0x210, 2, 0, 2, 4, rows(&[5, 6])) else { panic!("whole") };
        assert_eq!(version, "2");
        assert_eq!(parts, [SubPageRows { count: 4, first: 0, rows: rows(&[5, 6, 7, 8]) }]);
        // Another page's sending is its own.
        assert_eq!(lists.note(0x211, 2, 0, 2, 4, rows(&[1, 2])), Noted::Partial);
        assert_eq!(lists.note(0x210, 3, 0, 1, 1, rows(&[1])), Noted::Whole(0x210, "3".to_owned(), vec![SubPageRows { count: 1, first: 0, rows: rows(&[1]) }]));
    }
}
