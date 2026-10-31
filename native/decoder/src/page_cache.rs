//! Each tab's cache of page source (REQ-1558, DEC-1552): which pages and
//! sub-pages of a live channel its decoder holds, how old each is and when
//! it was last drawn. The source itself, each sub-page's packets merged as
//! they came, is in the engine's store, and nothing here is an image.

use std::collections::BTreeMap;

/// The most entries: the owner's "wall limit 128 tiles".
pub const MAX_CACHED_PAGES: usize = 128;
/// How long an entry stands, on the engine's monotonic clock, in
/// microseconds: the owner's "dump items from the cache after 24 hours".
pub const CACHED_PAGE_LIFETIME_US: u64 = 24 * 60 * 60 * 1_000_000;

const _: () = assert!(MAX_CACHED_PAGES == interface_core::wall::MAX_WALL_TILES);

/// Whether entries outlive a change of what is on screen: without the
/// `page-cache` feature nothing is kept, and each tile is filled every time
/// it is shown (REQ-1558 item 7).
const KEEPS: bool = cfg!(feature = "page-cache");

#[derive(Clone, Copy, Debug)]
struct Entry {
    stored_us: u64,
    /// Which draw last used it.
    used: u64,
}

/// The entries of one tab, by page and sub-code.
#[derive(Debug, Default)]
pub struct PageCache {
    entries: BTreeMap<(u16, u16), Entry>,
    draws: u64,
}

impl PageCache {
    /// A frame of `page`'s sub-page came at `now_us`: its entry is that
    /// frame, merged as the store merged it. A new entry in a full cache
    /// drops the one drawn longest ago.
    pub fn store(&mut self, page: u16, subcode: u16, now_us: u64) {
        let key = (page, subcode);
        if !self.entries.contains_key(&key) && self.entries.len() >= MAX_CACHED_PAGES {
            let oldest = self.entries.iter().min_by_key(|(_, entry)| entry.used).map(|(key, _)| *key);
            if let Some(oldest) = oldest {
                self.entries.remove(&oldest);
            }
        }
        self.draws = self.draws.saturating_add(1);
        let draws = self.draws;
        let entry = self.entries.entry(key).or_insert(Entry { stored_us: now_us, used: draws });
        entry.stored_us = now_us;
    }

    /// `page`'s sub-page is about to be drawn or shown at `now_us`: whether
    /// it may be. An entry past its lifetime is dropped here and nowhere
    /// else, so nothing expires while nothing is drawn.
    pub fn draw(&mut self, page: u16, subcode: u16, now_us: u64) -> bool {
        let key = (page, subcode);
        if !self.fresh(page, subcode, now_us) {
            self.entries.remove(&key);
            return false;
        }
        self.draws = self.draws.saturating_add(1);
        let draws = self.draws;
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.used = draws;
        }
        true
    }

    /// Whether an entry stands for `page`'s sub-page at `now_us`.
    #[must_use]
    pub fn fresh(&self, page: u16, subcode: u16, now_us: u64) -> bool {
        self.entries.get(&(page, subcode)).is_some_and(|entry| now_us.saturating_sub(entry.stored_us) <= CACHED_PAGE_LIFETIME_US)
    }

    /// What is on screen changed: without the feature nothing outlives it.
    pub fn screen_changed(&mut self) {
        if !KEEPS {
            self.entries.clear();
        }
    }

    /// The entries held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether none is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: u16 = 0x100;
    const SUB: u16 = 1;

    // REQ-1558 item 3: the 129th entry drops the one drawn longest ago, so
    // a page drawn lately stays, and the cache's size is its bound.
    #[test]
    fn the_129th_entry_drops_the_least_recently_drawn() {
        let mut cache = PageCache::default();
        for number in 0..u16::try_from(MAX_CACHED_PAGES).unwrap() {
            cache.store(PAGE + number, SUB, 0);
        }
        assert!(cache.draw(PAGE, SUB, 0), "the first, drawn last");
        cache.store(0x800, SUB, 0);
        assert_eq!(cache.len(), MAX_CACHED_PAGES, "the bound");
        assert!(cache.fresh(PAGE, SUB, 0), "drawn lately, it stays");
        assert!(!cache.fresh(PAGE + 1, SUB, 0), "the oldest drawn went");
        assert!(cache.fresh(0x800, SUB, 0));
    }

    // REQ-1558 item 4: one millisecond past 24 hours is dropped as it is
    // about to be drawn, and one millisecond short is drawn.
    #[test]
    fn an_entry_past_its_lifetime_is_dropped_when_drawn() {
        const MILLISECOND: u64 = 1_000;
        let mut cache = PageCache::default();
        cache.store(PAGE, SUB, 0);
        assert!(cache.draw(PAGE, SUB, CACHED_PAGE_LIFETIME_US - MILLISECOND), "a millisecond short");
        assert!(!cache.draw(PAGE, SUB, CACHED_PAGE_LIFETIME_US + MILLISECOND), "a millisecond past");
        assert_eq!(cache.len(), 0, "dropped");
    }

    // REQ-1558 item 4: nothing expires while nothing is drawn.
    #[test]
    fn nothing_expires_while_nothing_is_drawn() {
        let mut cache = PageCache::default();
        cache.store(PAGE, SUB, 0);
        assert!(!cache.fresh(PAGE, SUB, 10 * CACHED_PAGE_LIFETIME_US), "no use now");
        assert_eq!(cache.len(), 1, "but it is still there");
    }

    // REQ-1558 item 2: a new frame renews the entry's time.
    #[test]
    fn a_frame_renews_its_entry() {
        let mut cache = PageCache::default();
        cache.store(PAGE, SUB, 0);
        cache.store(PAGE, SUB, CACHED_PAGE_LIFETIME_US);
        assert_eq!(cache.len(), 1);
        assert!(cache.draw(PAGE, SUB, 2 * CACHED_PAGE_LIFETIME_US));
    }
}
