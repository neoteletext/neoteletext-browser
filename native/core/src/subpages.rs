//! A channel's sub-pages as the trusted core keeps them (decisions 111,
//! 112 and 119): each page's count as its decoder last said, and the parts
//! of the lists of sub-pages its fetcher read, `NNN/P.sub` (REQ-1888,
//! DOC-1869), or its live connection sent, for the pages the user looks
//! at. A list names every sub-page of a page whose sub-pages are files of
//! their own, held or not, up to the standard's 8,190 (the engine's
//! REQ-NUM-030), in parts of [`ROWS_PER_PART`] rows, and the core holds the
//! parts it has asked for: its count sizes the sub-page list, the parts its
//! rows fall in are asked for as it scrolls, stepping takes the next row,
//! and a sub-page's own file is asked for only when it is shown or next to
//! the one shown (decision 112 item 3).
//!
//! Everything here came from a fetcher or a decoder, so is untrusted:
//! counts are bounded by the standard's maximum, every title is cleaned, a
//! part must start at a part's first place and ascend, a part of another
//! version replaces what was held, and at most [`CATALOGUES_KEPT`] lists are
//! held a channel, the least recently used forgotten first.

use std::collections::{BTreeMap, BTreeSet};

pub mod list_state;

use list_state::{ListEvent, ListState, list_transition};

use neoteletext_core::limits::MAX_SUBPAGES;
use neoteletext_web::subpages::{PartNumber, parts_of_rows};
use protocol::fetcher::{MAX_LIST_PARTS, MAX_LIST_VERSION, ROWS_PER_PART};
use protocol::subpages::{SubPageRange, SubPageRows};
use protocol::{MAX_PAGE_TITLE, clean};

const _: () = assert!(ROWS_PER_PART == neoteletext_web::subpages::ROWS_PER_PART, "the engine's rows of a part");
const _: () = assert!(MAX_LIST_PARTS == neoteletext_web::subpages::MAX_PARTS, "the engine's parts of a list");
const _: () = assert!(MAX_LIST_VERSION == neoteletext_web::subpages::MAX_VALIDATOR, "the engine's longest version");

/// The lists that fit the budget when each is as long as a list can be: a
/// budget, not a maximum (decision 112 item 2). Each is at most 8,190
/// sub-codes and titles of a row, about 350 kilobytes with every part held.
pub const CATALOGUES_KEPT: usize = 16;
/// The rows of lists held a channel, in all: what [`CATALOGUES_KEPT`] whole
/// lists hold, so the memory is bounded as it was, and a channel of many
/// short lists keeps them all (DEC-6358: a live channel sends a page's
/// list once, so one forgotten cannot be asked for again, and its page
/// would be listed from what the decoder happens to hold). Over the
/// budget the least recently used list is forgotten, never the last.
pub const ROWS_KEPT: usize = CATALOGUES_KEPT * MAX_SUBPAGES;

/// The sub-pages either side of the one shown whose files are fetched
/// with it, so that stepping to them is at once (decision 112 item 3;
/// `STANDARD-LIMITS.md` question 4 suggests 2; one keeps a held key's
/// requests within the engine's queue of 16).
pub const TUNED_EITHER_SIDE: usize = 1;

/// One page's list as far as it is held: its number of sub-pages and its
/// version, from whichever part came first, and the parts that came, each
/// its sub-codes in ascending order with their titles, cleaned (the
/// engine's decision E.132 item 1).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalogue {
    total: u16,
    version: String,
    parts: BTreeMap<u8, Vec<(u16, String)>>,
}

impl Catalogue {
    /// How many sub-pages it lists, at most 8,190, held or not.
    #[must_use]
    pub const fn count(&self) -> u16 {
        self.total
    }

    /// The place of `subcode` in the page's order, from 0, when a part
    /// holding it is held.
    #[must_use]
    pub fn place_of(&self, subcode: u16) -> Option<u16> {
        self.parts.iter().find_map(|(&part, rows)| {
            let row = rows.binary_search_by_key(&subcode, |(listed, _)| *listed).ok()?;
            u16::try_from(PartNumber::new(usize::from(part))?.first_place().checked_add(row)?).ok()
        })
    }

    /// How many rows of it are held.
    fn rows_held(&self) -> usize {
        self.parts.values().map(Vec::len).sum()
    }

    /// The sub-code at `place`, from 0, when its part is held.
    #[must_use]
    pub fn subcode_at(&self, place: usize) -> Option<u16> {
        let part = PartNumber::of_place(place)?;
        let row = place.checked_sub(part.first_place())?;
        self.parts.get(&part.number())?.get(row).map(|(subcode, _)| *subcode)
    }

    /// The sub-page after (`forward`) or before `current`, round the list
    /// as a set's sub-page keys go; from the first when `current` is not
    /// held; none with fewer than two, or when the part it is in is not
    /// held.
    #[must_use]
    pub fn step(&self, current: u16, forward: bool) -> Option<u16> {
        let count = usize::from(self.total);
        if count < 2 {
            return None;
        }
        let next = match self.place_of(current).map(usize::from) {
            Some(place) if forward => place.saturating_add(1).checked_rem(count).unwrap_or(0),
            Some(place) => place.checked_sub(1).unwrap_or(count.saturating_sub(1)),
            None => 0,
        };
        self.subcode_at(next)
    }

    /// The sub-pages within `either_side` places of `subcode`, round the
    /// list, `subcode` first: those whose files are fetched together, of
    /// the parts held.
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

    /// The parts holding the places within `either_side` of `subcode`'s,
    /// round the list, that are not held: those stepping and the files
    /// fetched with the one shown will want. None while its own part is
    /// not held.
    #[must_use]
    pub fn parts_around(&self, subcode: u16, either_side: usize) -> Vec<u8> {
        let count = usize::from(self.total);
        let Some(place) = self.place_of(subcode).map(usize::from).filter(|_| count > 0) else { return Vec::new() };
        let places = (1..=either_side.min(count))
            .flat_map(|distance| [place.saturating_add(distance), place.saturating_add(count).saturating_sub(distance)])
            .filter_map(|place| place.checked_rem(count));
        let parts: BTreeSet<u8> = places.filter_map(PartNumber::of_place).map(PartNumber::number).collect();
        parts.into_iter().filter(|part| !self.parts.contains_key(part)).collect()
    }

    /// The parts `range`'s rows fall in that are not held (REQ-1888).
    #[must_use]
    pub fn parts_wanted(&self, range: SubPageRange) -> Vec<u8> {
        let first = usize::from(range.first.min(self.total));
        let count = usize::from(range.count).min(usize::from(self.total).saturating_sub(first));
        parts_of_rows(first, count).map(PartNumber::number).filter(|part| !self.parts.contains_key(part)).collect()
    }

    /// The rows `range` asks for, as the interface shows them: those held,
    /// from its first, up to the first not held.
    #[must_use]
    pub fn rows(&self, range: SubPageRange) -> SubPageRows {
        let count = self.total;
        let first = range.first.min(count);
        let rows = (usize::from(first)..usize::from(count))
            .take(usize::from(range.count))
            .map_while(|place| {
                let part = PartNumber::of_place(place)?;
                self.parts.get(&part.number())?.get(place.checked_sub(part.first_place())?).cloned()
            })
            .collect();
        SubPageRows { count, first, rows }
    }
}

/// Why a part of a list was refused: it is ignored, and the page is listed
/// from what it held and what its decoder holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// A part that does not start at a part's first place, holds more rows
    /// than a part, runs past the list's count, or whose sub-codes do not
    /// ascend (DOC-1869).
    Misshapen,
}

/// What a part of a list did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Arrived {
    /// The page's count to show, when it changed.
    pub count: Option<u16>,
    /// The parts held before that were of another version, forgotten and to
    /// be asked for again (DOC-1869).
    pub again: Vec<u8>,
}

/// A channel's sub-pages: each page's count as its decoder said, and the
/// lists its fetcher read.
#[derive(Debug, Default)]
pub struct SubPages {
    /// Each page's count as the channel's index declares it (DEC-6358):
    /// replaced whole by each index, and never changed by what a decoder
    /// holds.
    declared: BTreeMap<u16, u16>,
    /// Each page with two or more sub-pages stored, by its decoder's count.
    stored: BTreeMap<u16, u16>,
    /// Where each page's list stands (the list machine, `list_state`), and
    /// the count it gave while it is or was held: kept when the list
    /// itself is forgotten, so the page list's mark stays.
    lists: BTreeMap<u16, (ListState, u16)>,
    /// The lists held, the most recently used last.
    held: Vec<(u16, Catalogue)>,
    /// The parts asked for and not answered, or that failed, by page: each
    /// asked once until its page opens again.
    asked: BTreeSet<(u16, u8)>,
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

    /// The channel's index declares each page's count of sub-pages:
    /// `counts` replaces what the last index declared, a count of 0 being
    /// a page the index says nothing of. Returns the pages whose count to
    /// show changed, each with its new count.
    pub fn declare(&mut self, counts: &[(u16, u16)]) -> Vec<(u16, u16)> {
        let pages: BTreeSet<u16> = self.declared.keys().copied().chain(counts.iter().map(|&(page, _)| page)).collect();
        let before: Vec<(u16, u16)> = pages.iter().map(|&page| (page, self.count(page))).collect();
        let most = u16::try_from(MAX_SUBPAGES).unwrap_or(u16::MAX);
        self.declared = counts.iter().filter(|&&(_, count)| count > 0).map(|&(page, count)| (page, count.min(most))).collect();
        before.into_iter().filter_map(|(page, was)| Some((page, self.count(page))).filter(|&(_, now)| now != was)).collect()
    }

    /// How many sub-pages `page` has, to show: what the channel says, its
    /// list's count first and its index's next, and only for a page the
    /// channel says nothing of, what its decoder holds; 0 for none
    /// (DEC-6358 item 1).
    #[must_use]
    pub fn count(&self, page: u16) -> u16 {
        let listed = self.lists.get(&page).filter(|(state, _)| matches!(state, ListState::Held | ListState::Forgotten)).map(|(_, count)| count);
        let said = listed.or_else(|| self.declared.get(&page)).or_else(|| self.stored.get(&page));
        said.copied().filter(|&count| count >= 2).unwrap_or(0)
    }

    /// The list of `page`, when any of it is held: used now, so the last to
    /// be forgotten.
    pub fn catalogue(&mut self, page: u16) -> Option<&Catalogue> {
        let place = self.held.iter().position(|(held, _)| *held == page)?;
        let used = self.held.remove(place);
        self.held.push(used);
        self.held.last().map(|(_, catalogue)| catalogue)
    }

    /// Whether part `part` of `page`'s list should be asked for now: not
    /// when the page has no folder, nor while it was asked since its page
    /// last opened. One held is asked for again only by [`Self::again`],
    /// so a changed part is seen (the request is conditional).
    pub fn ask(&mut self, page: u16, part: u8) -> bool {
        if self.list_state(page) == ListState::Missing {
            return false;
        }
        let asked = self.asked.insert((page, part));
        if asked {
            self.list_event(page, ListEvent::Asked);
        }
        asked
    }

    /// `page` opens: every part of its list may be asked for again.
    pub fn again(&mut self, page: u16) {
        self.asked.retain(|&(asked, _)| asked != page);
        self.list_event(page, ListEvent::Reopened);
    }

    /// Where `page`'s list stands.
    #[must_use]
    pub fn list_state(&self, page: u16) -> ListState {
        self.lists.get(&page).map_or(ListState::Unknown, |&(state, _)| state)
    }

    /// Gives `event` to `page`'s list machine. A list not held or
    /// remembered gives no count.
    fn list_event(&mut self, page: u16, event: ListEvent) {
        let (state, count) = self.lists.get(&page).copied().unwrap_or_default();
        let next = list_transition(state, event);
        let count = if matches!(next, ListState::Held | ListState::Forgotten) { count } else { 0 };
        if next == ListState::Unknown {
            self.lists.remove(&page);
        } else {
            self.lists.insert(page, (next, count));
        }
    }

    /// A part of `page`'s list could not be fetched.
    pub fn failed(&mut self, page: u16) {
        self.list_event(page, ListEvent::PartFailed);
    }

    /// Part `part` of `page`'s list does not exist. For part 0 the page has
    /// no folder (DOC-1869): it is never asked for again, and its
    /// sub-pages are what its decoder holds. Returns the count to show when
    /// that changed it. Another part stays asked, as one that failed.
    pub fn missing(&mut self, page: u16, part: u8) -> Option<u16> {
        if part != PartNumber::FIRST.number() {
            return None;
        }
        let before = self.count(page);
        self.asked.retain(|&(asked, _)| asked != page);
        self.list_event(page, ListEvent::Absent);
        self.held.retain(|(held, _)| *held != page);
        let after = self.count(page);
        (after != before).then_some(after)
    }

    /// A part of `page`'s list, of `version`, at a part's first place, in
    /// any order: held with the parts of the same version and count, and
    /// replacing those of another, which are to be asked for again.
    ///
    /// # Errors
    /// When the part is not the shape of a part; nothing changes.
    pub fn part(&mut self, page: u16, version: &str, part: SubPageRows) -> Result<Arrived, Refused> {
        let first = usize::from(part.first);
        let number = PartNumber::of_place(first).filter(|number| number.first_place() == first).ok_or(Refused::Misshapen)?;
        let ascending = part.rows.windows(2).all(|pair| matches!(pair, [(before, _), (after, _)] if before < after));
        let fits = part.rows.len() <= ROWS_PER_PART && first.saturating_add(part.rows.len()) <= usize::from(part.count);
        if !ascending || !fits || usize::from(part.count) > MAX_SUBPAGES {
            return Err(Refused::Misshapen);
        }
        self.asked.remove(&(page, number.number()));
        let before = self.count(page);
        let mut catalogue = match self.held.iter().position(|(held, _)| *held == page) {
            Some(place) => self.held.remove(place).1,
            None => Catalogue::default(),
        };
        let mut again = Vec::new();
        if catalogue.parts.is_empty() || catalogue.version != version || catalogue.total != part.count {
            again = catalogue.parts.keys().copied().filter(|&held| held != number.number()).collect();
            catalogue = Catalogue { total: part.count, version: version.to_owned(), parts: BTreeMap::new() };
        }
        // Each title is cleaned as a page's title is.
        let rows = part.rows.into_iter().map(|(subcode, title)| (subcode, clean(&title, MAX_PAGE_TITLE))).collect();
        catalogue.parts.insert(number.number(), rows);
        self.list_event(page, ListEvent::Arrived);
        self.lists.insert(page, (ListState::Held, catalogue.total));
        self.held.push((page, catalogue));
        while self.held.len() > 1 && self.held.iter().map(|(_, held)| held.rows_held()).sum::<usize>() > ROWS_KEPT {
            let (forgotten, _) = self.held.remove(0);
            self.list_event(forgotten, ListEvent::Evicted);
        }
        let after = self.count(page);
        Ok(Arrived { count: (after != before).then_some(after), again })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]
mod tests {
    use super::*;
    use neoteletext_core::limits::from_rank;

    /// The list of a page of `total` sub-pages, every sub-code but 0000
    /// and 3F7F in ascending order.
    fn subcodes(total: u16) -> Vec<u16> {
        (1..=total).map(from_rank).collect()
    }

    /// Part `part` of the list of `subcodes`.
    fn part(subcodes: &[u16], part: usize) -> SubPageRows {
        let count = u16::try_from(subcodes.len()).unwrap();
        let rows = subcodes.chunks(ROWS_PER_PART).nth(part).unwrap_or_default();
        SubPageRows {
            count,
            first: u16::try_from(part * ROWS_PER_PART).unwrap(),
            rows: rows.iter().map(|&subcode| (subcode, format!("Story {subcode:04X}"))).collect(),
        }
    }

    /// Decision 119, REQ-1888 (the engine's REQ-NUM-020 and REQ-NUM-030): a
    /// list of the standard's 8,190 sub-pages, its 128 parts in any order,
    /// is counted, stepped round and listed by range.
    #[test]
    fn a_list_of_the_standards_maximum_is_held_and_stepped() {
        let most = u16::try_from(MAX_SUBPAGES).unwrap();
        let all = subcodes(most);
        let mut subpages = SubPages::default();
        let first = subpages.part(0x210, "v1", part(&all, 0)).unwrap();
        assert_eq!(first.count, Some(most), "the first part sizes the list");
        for number in (1..MAX_LIST_PARTS).rev() {
            assert_eq!(subpages.part(0x210, "v1", part(&all, number)), Ok(Arrived::default()));
        }
        let catalogue = subpages.catalogue(0x210).unwrap();
        let last = all[usize::from(most) - 1];
        assert_eq!(catalogue.step(last, true), Some(all[0]), "round from the last to the first");
        assert_eq!(catalogue.step(all[0], false), Some(last));
        assert_eq!(catalogue.place_of(all[7_999]), Some(7_999));
        let rows = catalogue.rows(SubPageRange { first: 8_188, count: 64 });
        assert_eq!((rows.count, rows.first, rows.rows.len()), (most, 8_188, 2));
        assert_eq!(catalogue.around(last, TUNED_EITHER_SIDE), [last, all[0], all[usize::from(most) - 2]]);
        assert_eq!(catalogue.parts_wanted(SubPageRange { first: 0, count: most }), [] as [u8; 0], "every part held");
    }

    /// REQ-1888's verification: a list of 8,190 of which part 0 is held
    /// shows rows 4,020 to 4,042 by asking for parts 62 and 63 alone; rows
    /// are given up to the first not held; stepping off a part not held
    /// goes nowhere, and the part beside it is the one wanted.
    #[test]
    fn rows_ask_for_the_parts_they_fall_in() {
        let all = subcodes(8_190);
        let mut subpages = SubPages::default();
        subpages.part(0x210, "v1", part(&all, 0)).unwrap();
        let range = SubPageRange { first: 4_020, count: 23 };
        let catalogue = subpages.catalogue(0x210).unwrap();
        assert_eq!(catalogue.parts_wanted(range), [62, 63]);
        assert!(catalogue.rows(range).rows.is_empty(), "nothing held there yet");
        assert_eq!(catalogue.step(all[63], true), None, "the next row is in part 1");
        assert_eq!(catalogue.parts_around(all[63], 2), [1], "part 1 ahead");
        assert_eq!(catalogue.parts_around(all[0], 2), [127], "round to the last behind the first");
        assert_eq!(catalogue.parts_around(all[10], 2), [] as [u8; 0], "within part 0");
        subpages.part(0x210, "v1", part(&all, 62)).unwrap();
        let catalogue = subpages.catalogue(0x210).unwrap();
        assert_eq!(catalogue.parts_wanted(range), [63]);
        assert_eq!(catalogue.rows(range).rows.len(), 4_032 - 4_020, "up to the end of part 62");
    }

    /// DOC-1869: a part of another version replaces what was held, the
    /// parts of the old one to be asked for again; parts not the shape of
    /// a part are refused and change nothing.
    #[test]
    fn a_part_of_another_version_replaces_the_list() {
        let all = subcodes(200);
        let mut subpages = SubPages::default();
        subpages.part(0x210, "v1", part(&all, 0)).unwrap();
        subpages.part(0x210, "v1", part(&all, 1)).unwrap();
        let arrived = subpages.part(0x210, "v2", part(&all, 2)).unwrap();
        assert_eq!(arrived.again, [0, 1]);
        let catalogue = subpages.catalogue(0x210).unwrap();
        assert_eq!(catalogue.subcode_at(0), None, "the old version's parts are gone");
        assert_eq!(catalogue.subcode_at(128), Some(all[128]));
        let misplaced = SubPageRows { first: 1, ..part(&all, 0) };
        assert_eq!(subpages.part(0x210, "v2", misplaced), Err(Refused::Misshapen), "not at a part's first place");
        let mut backwards = part(&all, 0);
        backwards.rows.reverse();
        assert_eq!(subpages.part(0x210, "v2", backwards), Err(Refused::Misshapen), "descending");
        let mut repeated = part(&all, 0);
        repeated.rows[1].0 = repeated.rows[0].0;
        assert_eq!(subpages.part(0x210, "v2", repeated), Err(Refused::Misshapen), "a sub-code twice");
        let mut long = part(&all, 0);
        long.rows.push((0x3F7E, String::new()));
        assert_eq!(subpages.part(0x210, "v2", long), Err(Refused::Misshapen), "a 65th row");
        assert_eq!(subpages.catalogue(0x210).unwrap().subcode_at(0), None, "none of them was taken");
    }

    /// DOC-1869: a page whose part 0 does not exist has no folder and is
    /// never asked again; another part missing stays asked, as one that
    /// failed, until its page opens again.
    #[test]
    fn a_page_with_no_folder_is_not_asked_again() {
        let mut subpages = SubPages::default();
        assert!(subpages.ask(0x300, 0));
        assert!(!subpages.ask(0x300, 0), "one request out at a time");
        subpages.missing(0x300, 0);
        subpages.again(0x300);
        assert!(!subpages.ask(0x300, 0), "no folder");
        assert_eq!(subpages.stored(0x300, 3), Some(3), "its decoder's count stands");
        assert!(subpages.ask(0x310, 4));
        assert_eq!(subpages.missing(0x310, 4), None);
        assert!(!subpages.ask(0x310, 4), "not asked again while open");
        subpages.again(0x310);
        assert!(subpages.ask(0x310, 4), "asked again when its page opens");
    }

    /// Decision 112 item 2 as DEC-6358 keeps it: lists are a budget of
    /// rows, the least recently used forgotten first; a forgotten page
    /// keeps its count; and a channel of many short lists keeps them all,
    /// which the budget of sixteen lists it replaces did not.
    #[test]
    fn lists_are_a_cache() {
        let two = subcodes(2);
        let mut subpages = SubPages::default();
        for page in 0x100..0x100 + 100 {
            subpages.part(page, "v", part(&two, 0)).unwrap();
        }
        assert!((0x100..0x100 + 100).all(|page| subpages.catalogue(page).is_some()), "a hundred short lists are all kept");
        // Whole parts of long lists, until the rows held pass the budget.
        let long = subcodes(u16::try_from(MAX_SUBPAGES).unwrap());
        let mut subpages = SubPages::default();
        let parts_to_fill = ROWS_KEPT / ROWS_PER_PART;
        let mut held = 0;
        'pages: for page in 0x100_u16.. {
            for number in 0..MAX_LIST_PARTS {
                let rows = part(&long, number);
                held += rows.rows.len();
                subpages.part(page, "v", rows).unwrap();
                if page == 0x102 && number == 0 {
                    // Used again after the second list, so not the first forgotten.
                    assert!(subpages.catalogue(0x100).is_some());
                }
                if held > ROWS_KEPT {
                    break 'pages;
                }
            }
        }
        assert!(parts_to_fill > 0 && subpages.catalogue(0x100).is_some(), "used since");
        assert!(subpages.catalogue(0x101).is_none(), "the least recently used, forgotten");
        assert_eq!(subpages.count(0x101), 8_190, "its count kept for the page list");
        let titled = SubPageRows { count: 1, first: 0, rows: vec![(1, "a\u{202E}b".into())] };
        subpages.part(0x400, "v", titled).unwrap();
        assert_eq!(subpages.catalogue(0x400).unwrap().rows(SubPageRange { first: 0, count: 1 }).rows[0].1, "ab", "titles cleaned");
        assert_eq!(subpages.count(0x400), 0, "one sub-page is none to list");
    }

    /// DEC-6358 item 1: the channel's word decides a page's count. Its
    /// index declares it, a list refines it, and what a decoder holds is
    /// used only for a page the channel says nothing of.
    #[test]
    fn the_channels_count_is_never_overridden_by_what_a_decoder_holds() {
        let mut subpages = SubPages::default();
        assert_eq!(subpages.declare(&[(0x101, 10), (0x102, 5), (0x103, 1), (0x104, 0)]), [(0x101, 10), (0x102, 5)]);
        assert_eq!((subpages.count(0x101), subpages.count(0x103), subpages.count(0x104)), (10, 0, 0));
        // The decoder holds one of ten: the count stays ten (fault 1).
        assert_eq!(subpages.stored(0x101, 1), None);
        assert_eq!(subpages.stored(0x101, 3), None);
        assert_eq!(subpages.count(0x101), 10);
        // A page the index says has one sub-page has none to list, whatever is held.
        assert_eq!(subpages.stored(0x103, 4), None);
        assert_eq!(subpages.count(0x103), 0);
        // A page the index says nothing of is counted from the decoder.
        assert_eq!(subpages.stored(0x104, 4), Some(4));
        // A list refines the index's count.
        let twelve = subcodes(12);
        assert_eq!(subpages.part(0x101, "v", part(&twelve, 0)).unwrap().count, Some(12));
        // A new index replaces the last: a page it drops falls back, one it changes follows.
        assert_eq!(subpages.declare(&[(0x102, 7)]), [(0x102, 7), (0x103, 4)], "and one it no longer speaks of is the decoder's again");
        assert_eq!((subpages.count(0x101), subpages.count(0x102)), (12, 7), "the list's count stands without the index's");
    }

    /// DEC-6358 item 4: where a page's list stands is one state, moved by
    /// what happens to it, and the count and the asking follow that state.
    #[test]
    fn a_pages_list_moves_through_its_states() {
        use list_state::ListState as S;
        let three = subcodes(3);
        let mut subpages = SubPages::default();
        assert_eq!(subpages.list_state(0x200), S::Unknown);
        assert!(subpages.ask(0x200, 0));
        assert_eq!(subpages.list_state(0x200), S::Fetching, "asked for");
        subpages.failed(0x200);
        assert_eq!(subpages.list_state(0x200), S::Failed, "it could not be fetched");
        assert!(!subpages.ask(0x200, 0), "not asked again while its page stays open");
        subpages.again(0x200);
        assert_eq!(subpages.list_state(0x200), S::Unknown, "its page opens again");
        assert!(subpages.ask(0x200, 0));
        subpages.part(0x200, "v", part(&three, 0)).unwrap();
        assert_eq!((subpages.list_state(0x200), subpages.count(0x200)), (S::Held, 3));
        subpages.failed(0x200);
        assert_eq!(subpages.list_state(0x200), S::Held, "a later part failing leaves what is held");
        assert_eq!(subpages.missing(0x200, 0), Some(0), "the channel no longer has a list for it");
        assert_eq!((subpages.list_state(0x200), subpages.count(0x200)), (S::Missing, 0));
        assert!(!subpages.ask(0x200, 0), "and it is not asked for again");
        subpages.again(0x200);
        assert!(!subpages.ask(0x200, 0), "even when its page opens again");
        subpages.part(0x200, "w", part(&three, 0)).unwrap();
        assert_eq!(subpages.list_state(0x200), S::Held, "until the channel sends one");
    }
}
