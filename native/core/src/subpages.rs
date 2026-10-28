//! A channel's sub-pages as the trusted core keeps them (decisions 111,
//! 112 and 119): each page's count as its decoder last said, and the
//! catalogues of sub-pages its fetcher read, `NNN.sub` (the engine's
//! REQ-WEB-034), for the pages the user looks at. A catalogue lists every
//! sub-page of a page served as files, held or not, up to the standard's
//! 8,190 (the engine's REQ-NUM-030); the core steps through it, answers
//! the sub-page list's rows from it, and asks for a sub-page's own file
//! only when it is shown or next to the one shown (decision 112 item 3).
//!
//! Everything here came from a fetcher or a decoder, so is untrusted:
//! counts are bounded by the standard's maximum, every title is cleaned,
//! a catalogue's parts must come in order, and at most
//! [`CATALOGUES_KEPT`] catalogues are held a channel, the least recently
//! used forgotten first.

use std::collections::{BTreeMap, BTreeSet};

use neoteletext_core::limits::MAX_SUBPAGES;
use protocol::subpages::{SubPageRange, SubPageRows};
use protocol::{MAX_PAGE_TITLE, clean};

/// The catalogues held a channel: a budget, not a maximum (decision 112
/// item 2). Each is at most 8,190 sub-codes and titles of a row, about
/// 350 kilobytes as the engine's reader reads them and 1.3 megabytes at
/// the protocol's bound; a page whose catalogue was forgotten has it
/// fetched again when it next opens.
pub const CATALOGUES_KEPT: usize = 16;

/// The sub-pages either side of the one shown whose files are fetched
/// with it, so that stepping to them is at once (decision 112 item 3;
/// `STANDARD-LIMITS.md` question 4 suggests 2; one keeps a held key's
/// requests within the engine's queue of 16).
pub const TUNED_EITHER_SIDE: usize = 1;

/// One page's catalogue: its sub-codes in the page's order, each with its
/// title, cleaned (the engine's decision E.132 item 1).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalogue {
    entries: Vec<(u16, String)>,
}

impl Catalogue {
    /// How many sub-pages it lists, at most 8,190.
    #[must_use]
    pub fn count(&self) -> u16 {
        u16::try_from(self.entries.len()).unwrap_or(u16::MAX)
    }

    /// The place of `subcode` in the page's order, from 0.
    #[must_use]
    pub fn place_of(&self, subcode: u16) -> Option<u16> {
        self.entries.iter().position(|(listed, _)| *listed == subcode).and_then(|place| u16::try_from(place).ok())
    }

    /// The sub-code at `place`, from 0.
    #[must_use]
    pub fn subcode_at(&self, place: usize) -> Option<u16> {
        self.entries.get(place).map(|(subcode, _)| *subcode)
    }

    /// The sub-page after (`forward`) or before `current`, round the list
    /// as a set's sub-page keys go; from the first when `current` is not
    /// listed; none with fewer than two.
    #[must_use]
    pub fn step(&self, current: u16, forward: bool) -> Option<u16> {
        let count = self.entries.len();
        if count < 2 {
            return None;
        }
        let next = match self.entries.iter().position(|(listed, _)| *listed == current) {
            Some(place) if forward => place.saturating_add(1).checked_rem(count).unwrap_or(0),
            Some(place) => place.checked_sub(1).unwrap_or(count.saturating_sub(1)),
            None => 0,
        };
        self.subcode_at(next)
    }

    /// The sub-pages within `either_side` places of `subcode`, round the
    /// list, `subcode` first: those whose files are fetched together.
    #[must_use]
    pub fn around(&self, subcode: u16, either_side: usize) -> Vec<u16> {
        let mut around = vec![subcode];
        let (mut before, mut after) = (subcode, subcode);
        for _ in 0..either_side {
            for (current, forward) in [(&mut after, true), (&mut before, false)] {
                if let Some(next) = self.step(*current, forward) {
                    *current = next;
                    if !around.contains(&next) {
                        around.push(next);
                    }
                }
            }
        }
        around
    }

    /// The rows `range` asks for, as the interface shows them.
    #[must_use]
    pub fn rows(&self, range: SubPageRange) -> SubPageRows {
        let count = self.count();
        let first = range.first.min(count);
        let rows = self.entries.iter().skip(usize::from(first)).take(usize::from(range.count)).cloned().collect();
        SubPageRows { count, first, rows }
    }
}

/// Why a catalogue's part was refused: the catalogue is dropped, and the
/// page is listed from what its decoder holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// A part that does not follow the one before it.
    OutOfOrder,
}

/// A catalogue being received, part by part.
#[derive(Debug)]
struct Partial {
    page: u16,
    count: u16,
    entries: Vec<(u16, String)>,
}

/// A channel's sub-pages: each page's count as its decoder said, and the
/// catalogues its fetcher read.
#[derive(Debug, Default)]
pub struct SubPages {
    /// Each page with two or more sub-pages stored, by its decoder's count.
    stored: BTreeMap<u16, u16>,
    /// Each page whose catalogue answered, by its count: kept when the
    /// catalogue itself is forgotten, so the page list's mark stays.
    catalogued: BTreeMap<u16, u16>,
    /// The catalogues held, the most recently used last.
    held: Vec<(u16, Catalogue)>,
    /// The catalogue being received.
    partial: Option<Partial>,
    /// Pages whose channel has no catalogue: never asked again.
    missing: BTreeSet<u16>,
    /// Pages whose catalogue was asked for and has not answered.
    asked: BTreeSet<u16>,
}

impl SubPages {
    /// The decoder's count of `page`'s stored sub-pages; returns the
    /// count to show when it changed it.
    pub fn stored(&mut self, page: u16, count: u16) -> Option<u16> {
        let before = self.count(page);
        let count = count.min(u16::try_from(MAX_SUBPAGES).unwrap_or(u16::MAX));
        if count < 2 {
            self.stored.remove(&page);
        } else {
            self.stored.insert(page, count);
        }
        let after = self.count(page);
        (after != before).then_some(after)
    }

    /// How many sub-pages `page` has, to show: its catalogue's count when it
    /// has one, else what its decoder holds; 0 for none.
    #[must_use]
    pub fn count(&self, page: u16) -> u16 {
        self.catalogued.get(&page).or_else(|| self.stored.get(&page)).copied().filter(|&count| count >= 2).unwrap_or(0)
    }

    /// Every page with sub-pages to show, with its count.
    #[must_use]
    pub fn counts(&self) -> Vec<(u16, u16)> {
        let pages: BTreeSet<u16> = self.stored.keys().chain(self.catalogued.keys()).copied().collect();
        pages.into_iter().map(|page| (page, self.count(page))).filter(|&(_, count)| count > 0).collect()
    }

    /// The catalogue of `page`, when one is held: used now, so the last
    /// to be forgotten.
    pub fn catalogue(&mut self, page: u16) -> Option<&Catalogue> {
        let place = self.held.iter().position(|(held, _)| *held == page)?;
        let used = self.held.remove(place);
        self.held.push(used);
        self.held.last().map(|(_, catalogue)| catalogue)
    }

    /// Whether `page`'s catalogue should be asked for now, as its page
    /// opens: not when the channel has none for it, nor while a request is
    /// out. One held is asked for again, so a changed catalogue is seen
    /// (the request is conditional, REQ-WEB-004).
    pub fn ask(&mut self, page: u16) -> bool {
        if self.missing.contains(&page) || self.asked.contains(&page) {
            return false;
        }
        self.asked.insert(page)
    }

    /// The page has no catalogue: it is never asked for again, and its
    /// sub-pages are what its decoder holds. Returns the count to show
    /// when that changed it.
    pub fn missing(&mut self, page: u16) -> Option<u16> {
        let before = self.count(page);
        self.asked.remove(&page);
        self.missing.insert(page);
        self.catalogued.remove(&page);
        self.held.retain(|(held, _)| *held != page);
        if self.partial.as_ref().is_some_and(|partial| partial.page == page) {
            self.partial = None;
        }
        let after = self.count(page);
        (after != before).then_some(after)
    }

    /// The catalogue's request ended with no catalogue: it may be asked
    /// for again.
    pub fn unanswered(&mut self, page: u16) {
        self.asked.remove(&page);
    }

    /// A part of `page`'s catalogue, in order. Returns the page's count to
    /// show once the last part has come.
    ///
    /// # Errors
    /// When the part does not follow the one before it; what had come of
    /// the catalogue is dropped.
    pub fn part(&mut self, page: u16, part: SubPageRows) -> Result<Option<u16>, Refused> {
        let follows = part.first == 0
            || self
                .partial
                .as_ref()
                .is_some_and(|partial| partial.page == page && partial.count == part.count && usize::from(part.first) == partial.entries.len());
        if !follows {
            self.partial = None;
            return Err(Refused::OutOfOrder);
        }
        if part.first == 0 {
            self.partial = Some(Partial { page, count: part.count, entries: Vec::with_capacity(usize::from(part.count).min(MAX_SUBPAGES)) });
        }
        let Some(partial) = self.partial.as_mut() else { return Err(Refused::OutOfOrder) };
        // The reader bounded the part within the count; each title is
        // cleaned as a page's title is.
        partial.entries.extend(part.rows.into_iter().map(|(subcode, title)| (subcode, clean(&title, MAX_PAGE_TITLE))));
        if partial.entries.len() < usize::from(partial.count) {
            return Ok(None);
        }
        let Some(Partial { entries, .. }) = self.partial.take() else { return Ok(None) };
        Ok(Some(self.install(page, Catalogue { entries })))
    }

    /// `page`'s catalogue arrived whole: held, the least recently used
    /// forgotten past [`CATALOGUES_KEPT`]. Returns its count to show.
    fn install(&mut self, page: u16, catalogue: Catalogue) -> u16 {
        self.asked.remove(&page);
        self.missing.remove(&page);
        self.held.retain(|(held, _)| *held != page);
        if catalogue.entries.is_empty() {
            self.catalogued.remove(&page);
            return self.count(page);
        }
        self.catalogued.insert(page, catalogue.count());
        self.held.push((page, catalogue));
        if self.held.len() > CATALOGUES_KEPT {
            self.held.remove(0);
        }
        self.count(page)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]
mod tests {
    use super::*;

    fn part(count: u16, first: u16, subcodes: core::ops::Range<u16>) -> SubPageRows {
        SubPageRows { count, first, rows: subcodes.map(|subcode| (subcode, format!("Story {subcode}"))).collect() }
    }

    /// Decision 119 (the engine's REQ-NUM-020 and REQ-NUM-030): a
    /// catalogue of the standard's 8,190 sub-pages, in parts, is held
    /// whole, stepped round and listed by range.
    #[test]
    fn a_catalogue_of_the_standards_maximum_is_held_and_stepped() {
        let most = u16::try_from(MAX_SUBPAGES).unwrap();
        let mut subpages = SubPages::default();
        let mut first = 0;
        let mut answered = None;
        while first < most {
            let last = (first + 1024).min(most);
            answered = subpages.part(0x210, part(most, first, first + 1..last + 1)).unwrap();
            first = last;
        }
        assert_eq!(answered, Some(most));
        assert_eq!(subpages.count(0x210), most);
        let catalogue = subpages.catalogue(0x210).unwrap();
        assert_eq!(catalogue.step(most, true), Some(1), "round from the last to the first");
        assert_eq!(catalogue.step(1, false), Some(most));
        assert_eq!(catalogue.place_of(8_000), Some(7_999));
        let rows = catalogue.rows(SubPageRange { first: 8_188, count: 64 });
        assert_eq!((rows.count, rows.first, rows.rows.len()), (most, 8_188, 2));
        assert_eq!(catalogue.around(most, TUNED_EITHER_SIDE), [most, 1, most - 1]);
    }

    /// A part that does not follow the one before it drops the catalogue;
    /// a page with none is never asked for again.
    #[test]
    fn parts_come_in_order_and_a_missing_catalogue_is_not_asked_again() {
        let mut subpages = SubPages::default();
        assert_eq!(subpages.part(0x210, part(4, 0, 1..3)), Ok(None));
        assert_eq!(subpages.part(0x210, part(4, 3, 4..5)), Err(Refused::OutOfOrder), "a part skipped");
        assert_eq!(subpages.part(0x210, part(4, 2, 3..5)), Err(Refused::OutOfOrder), "nothing to follow now");
        assert!(subpages.catalogue(0x210).is_none());
        assert!(subpages.ask(0x300));
        assert!(!subpages.ask(0x300), "one request out at a time");
        subpages.missing(0x300);
        assert!(!subpages.ask(0x300));
        assert_eq!(subpages.stored(0x300, 3), Some(3), "its decoder's count stands");
    }

    /// Decision 112 item 2: catalogues are a budget, the least recently
    /// used forgotten first; a forgotten page keeps its count.
    #[test]
    fn catalogues_are_a_cache() {
        let mut subpages = SubPages::default();
        for page in 0..=u16::try_from(CATALOGUES_KEPT).unwrap() {
            let page = 0x100 + page;
            subpages.part(page, part(2, 0, 1..3)).unwrap();
            if page == 0x101 {
                // Used again, so not the first forgotten.
                assert!(subpages.catalogue(0x100).is_some());
            }
        }
        assert!(subpages.catalogue(0x100).is_some(), "used since");
        assert!(subpages.catalogue(0x101).is_none(), "the least recently used, forgotten");
        assert_eq!(subpages.count(0x101), 2, "its count kept for the page list");
        assert!(subpages.ask(0x101), "asked for again when its page opens");
        let titled = SubPageRows { count: 1, first: 0, rows: vec![(1, "a\u{202E}b".into())] };
        subpages.part(0x400, titled).unwrap();
        assert_eq!(subpages.catalogue(0x400).unwrap().rows(SubPageRange { first: 0, count: 1 }).rows[0].1, "ab", "titles cleaned");
        assert_eq!(subpages.count(0x400), 0, "one sub-page is none to list");
    }
}
