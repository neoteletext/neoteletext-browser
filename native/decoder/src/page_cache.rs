//! Each tab's cache of page source (REQ-1558, DEC-1552): which pages and
//! sub-pages of a live channel its decoder holds, how old each is and when
//! it was last drawn. The source itself, each sub-page's packets merged as
//! they came, is in the engine's store, and nothing here is an image.

use std::collections::BTreeMap;

use crate::animation::frames::Held;

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

/// The most bytes of picture source a tab keeps for back and forward
/// (renderer parity PAR-34): 32 MiB, the streams, sheets, objects and links
/// of the picture sub-pages its decoder has drawn. One more drops the
/// source drawn longest ago, as an entry past [`MAX_CACHED_PAGES`] does.
pub const MAX_KEPT_PICTURE_SOURCE_BYTES: usize = 33_554_432;

/// What a source costs beyond its bytes, for each record it keeps: the
/// record's own size in this process, so that many small records count.
const RECORD_COST: usize = size_of::<Held>();

/// One picture sub-page's source, as received (PAR-34): its sheet, its
/// stream from the first byte, how that stream is painted (its PAINT
/// record, DEC-7031 item 1), its objects, its cursor and its drawing
/// links. Its animation state is local and never kept (DEC-6587 item 3).
#[derive(Debug, Default)]
struct Source {
    sheet: Option<Held>,
    /// The PAINT record last received, which is the kept stream's only
    /// while it names that stream's language and generation.
    paint: Option<Held>,
    stream: Option<Held>,
    objects: Vec<Held>,
    cursor: Option<Held>,
    links: Option<Vec<(u16, String)>>,
    bytes: usize,
    stored_us: u64,
    used: u64,
}

/// The bytes a held record keeps.
fn held_bytes(held: &Held) -> usize {
    let data = match held {
        Held::Stream { data, .. } | Held::Sheet { data, .. } => data.len(),
        Held::Objects { objects, title, .. } => objects
            .iter()
            .map(|object| size_of::<naplps::Object>().saturating_add(object.label.len()).saturating_add(object.frames.len()))
            .fold(title.as_ref().map_or(0, String::len), usize::saturating_add),
        _ => 0,
    };
    data.saturating_add(RECORD_COST)
}

impl Source {
    fn count(&mut self) {
        let records = self.sheet.iter().chain(&self.paint).chain(&self.stream).chain(&self.objects).chain(&self.cursor);
        let links = self.links.iter().flatten().map(|(_, target)| target.len().saturating_add(RECORD_COST)).fold(0, usize::saturating_add);
        self.bytes = records.map(held_bytes).fold(links, usize::saturating_add);
    }

    /// Adds `data` at `offset` to a stream or sheet record of
    /// `generation`: a first byte begins it again, the next bytes in their
    /// place lengthen it, and anything else loses it.
    fn extend(kept: &mut Option<Held>, item: &Held) {
        let (Held::Stream { generation, offset, data, .. } | Held::Sheet { generation, offset, data, .. }) = item else { return };
        if *offset == 0 {
            *kept = Some(item.clone());
            return;
        }
        let place = |held: &Held| match held {
            Held::Stream { generation, data, .. } | Held::Sheet { generation, data, .. } => Some((*generation, data.len())),
            _ => None,
        };
        let follows = kept.as_ref().and_then(place).is_some_and(|(held, length)| held == *generation && usize::try_from(*offset).ok() == Some(length));
        match kept.as_mut() {
            Some(Held::Stream { data: held, .. } | Held::Sheet { data: held, .. }) if follows => held.extend_from_slice(data),
            _ => *kept = None,
        }
    }

    /// The PAINT record kept, when it is the kept stream's: of its
    /// language and its generation. A PAINT record comes before its
    /// stream's first byte, so the one kept may be the next stream's,
    /// which is not replayed with this one.
    fn paint_of_stream(&self) -> Option<&Held> {
        let Some(Held::Stream { language: of, generation: stream, .. }) = &self.stream else { return None };
        self.paint.as_ref().filter(|paint| matches!(paint, Held::Paint { language, generation, .. } if language == of && generation == stream))
    }
}

/// A kept picture's records, in the order they are applied, and its
/// page, sub-code and drawing links, if any came.
pub(crate) type Restored = (Vec<Held>, Option<(u16, u16, Vec<(u16, String)>)>);

/// The picture sources a tab keeps (renderer parity PAR-34), by page and
/// sub-code together (DEC-6599): each picture sub-page is kept as its
/// records, not as a renderer, under the same rule as the teletext pages,
/// [`CACHED_PAGE_LIFETIME_US`] and the one drawn longest ago dropped
/// first, within [`MAX_KEPT_PICTURE_SOURCE_BYTES`]. A kept picture shown
/// again is drawn from its records into a new terminal at once; the
/// channel sending the picture again replaces them.
#[derive(Debug, Default)]
pub struct PictureSources {
    sources: BTreeMap<(u16, u16), Source>,
    bytes: usize,
    uses: u64,
}

impl PictureSources {
    /// Keeps a record of a picture sub-page as it is applied at `now_us`.
    pub(crate) fn record(&mut self, item: &Held, now_us: u64) {
        if !KEEPS {
            return;
        }
        let Some(key) = item.key() else { return };
        self.uses = self.uses.saturating_add(1);
        let uses = self.uses;
        let source = self.sources.entry(key).or_default();
        match item {
            Held::Stream { generation, offset, .. } => {
                let renewed = *offset == 0 && !matches!(source.stream, Some(Held::Stream { generation: held, .. }) if held == *generation);
                Source::extend(&mut source.stream, item);
                // A new stream's objects come with it, after it.
                if renewed {
                    source.objects.retain(|held| matches!(held, Held::Objects { generation: of, .. } if of == generation));
                }
            }
            Held::Sheet { .. } => Source::extend(&mut source.sheet, item),
            Held::Objects { scope, .. } => {
                if *scope == crate::naplps::Scope::Whole {
                    source.objects.clear();
                }
                source.objects.push(item.clone());
            }
            Held::Cursor { .. } => source.cursor = Some(item.clone()),
            // Kept with the stream, so that a picture drawn again from its
            // source is replayed as it was first painted (DEC-7031 item 1).
            Held::Paint { .. } => source.paint = Some(item.clone()),
            _ => return,
        }
        source.stored_us = now_us;
        source.used = uses;
        self.settle(key);
    }

    /// Keeps a sub-page's drawing links, received whole.
    #[cfg(feature = "links")]
    pub(crate) fn record_links(&mut self, page: u16, sub: u16, links: &[(u16, String)]) {
        if !KEEPS {
            return;
        }
        if let Some(source) = self.sources.get_mut(&(page, sub)) {
            source.links = Some(links.to_vec());
            self.settle((page, sub));
        }
    }

    /// Counts the source of `key` again and holds every source to the
    /// budget, dropping those drawn longest ago first; a source larger
    /// than the whole budget is not kept.
    fn settle(&mut self, key: (u16, u16)) {
        if let Some(source) = self.sources.get_mut(&key) {
            source.count();
            if source.bytes > MAX_KEPT_PICTURE_SOURCE_BYTES {
                self.sources.remove(&key);
            }
        }
        self.bytes = self.sources.values().map(|source| source.bytes).fold(0, usize::saturating_add);
        while self.bytes > MAX_KEPT_PICTURE_SOURCE_BYTES {
            let oldest = self.sources.iter().filter(|(held, _)| **held != key).min_by_key(|(_, source)| source.used).map(|(held, _)| *held);
            let dropped = oldest.unwrap_or(key);
            let freed = self.sources.remove(&dropped).map_or(0, |source| source.bytes);
            self.bytes = self.bytes.saturating_sub(freed);
            if dropped == key {
                break;
            }
        }
    }

    /// The records to draw the kept picture of `page`'s sub-page `sub` (or
    /// with none, its first kept) again, at `now_us`, in the order they
    /// are applied, with its drawing links; `None` when none is kept or it
    /// is past its lifetime, when it is dropped.
    pub(crate) fn restore(&mut self, page: u16, sub: Option<u16>, now_us: u64) -> Option<Restored> {
        let key = match sub {
            Some(sub) if sub != neoteletext_core::limits::SUBCODE_ANY => (page, sub),
            _ => self.sources.keys().find(|(held, _)| *held == page).copied()?,
        };
        let source = self.sources.get(&key)?;
        source.stream.as_ref()?;
        if now_us.saturating_sub(source.stored_us) > CACHED_PAGE_LIFETIME_US {
            let freed = self.sources.remove(&key).map_or(0, |source| source.bytes);
            self.bytes = self.bytes.saturating_sub(freed);
            return None;
        }
        self.uses = self.uses.saturating_add(1);
        let uses = self.uses;
        let source = self.sources.get_mut(&key)?;
        source.used = uses;
        // The PAINT record goes before the stream it is for, as it came.
        let records = source.sheet.iter().chain(source.paint_of_stream()).chain(&source.stream).chain(&source.objects).chain(&source.cursor).cloned().collect();
        let links = source.links.clone().map(|links| (key.0, key.1, links));
        Some((records, links))
    }

    /// The bytes kept.
    #[cfg(all(test, feature = "page-cache"))]
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Whether a source is kept for `page`'s sub-page `sub`.
    #[cfg(all(test, feature = "page-cache"))]
    #[must_use]
    pub fn keeps(&self, page: u16, sub: u16) -> bool {
        self.sources.get(&(page, sub)).is_some_and(|source| source.stream.is_some())
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

    #[cfg(feature = "page-cache")]
    use crate::animation::frames::Language;

    #[cfg(feature = "page-cache")]
    fn stream(page: u16, generation: u16, offset: u32, data: Vec<u8>) -> Held {
        Held::Stream { language: Language::Regis, page, sub: SUB, generation, offset, data }
    }

    // PAR-34: a stream is kept from its first byte, its records joined in
    // order; a new generation replaces it; a gap loses it.
    // Without the feature nothing is kept (REQ-1558 item 7).
    #[cfg(feature = "page-cache")]
    #[test]
    fn a_stream_is_kept_whole_and_replaced_by_the_next_generation() {
        let mut sources = PictureSources::default();
        sources.record(&stream(PAGE, 1, 0, b"P[1,1]".to_vec()), 0);
        sources.record(&stream(PAGE, 1, 6, b"V[2,2]".to_vec()), 0);
        let (records, _) = sources.restore(PAGE, Some(SUB), 0).unwrap();
        assert!(matches!(records.as_slice(), [Held::Stream { data, .. }] if data == b"P[1,1]V[2,2]"));
        sources.record(&stream(PAGE, 2, 0, b"V[9,9]".to_vec()), 0);
        let (records, _) = sources.restore(PAGE, Some(SUB), 0).unwrap();
        assert!(matches!(records.as_slice(), [Held::Stream { generation: 2, data, .. }] if data == b"V[9,9]"));
        sources.record(&stream(PAGE, 2, 99, b"lost".to_vec()), 0);
        assert!(sources.restore(PAGE, Some(SUB), 0).is_none(), "a gap loses the stream");
        assert!(sources.restore(PAGE + 1, Some(SUB), 0).is_none(), "another page has none");
        assert!(sources.restore(PAGE, Some(SUB + 1), 0).is_none(), "nor another sub-code of the page (DEC-6599)");
    }

    // DEC-7031 item 1 (PAINT kept with the stream so a restored page
    // replays again), PAR-34: the PAINT record kept goes before the stream
    // it is for, and only while it names that stream's language and
    // generation; one for the next generation, or for the sub-page's other
    // language, is not given back with the kept stream.
    #[cfg(feature = "page-cache")]
    #[test]
    fn a_kept_paint_record_goes_before_its_stream_only_for_its_language_and_generation() {
        use crate::animation::frames::Paint;
        let paint =
            |language, generation| Held::Paint { language, page: PAGE, sub: SUB, generation, paint: Paint { line_speed: Some(1200), default_rate: None } };
        let mut sources = PictureSources::default();
        sources.record(&paint(Language::Regis, 1), 0);
        sources.record(&stream(PAGE, 1, 0, b"P[1,1]".to_vec()), 0);
        let (records, _) = sources.restore(PAGE, Some(SUB), 0).unwrap();
        assert!(
            matches!(records.as_slice(), [Held::Paint { language: Language::Regis, generation: 1, .. }, Held::Stream { generation: 1, .. }]),
            "the PAINT record first, then its stream: {records:?}"
        );
        // The next generation's PAINT record comes before its stream.
        sources.record(&paint(Language::Regis, 2), 0);
        let (records, _) = sources.restore(PAGE, Some(SUB), 0).unwrap();
        assert!(matches!(records.as_slice(), [Held::Stream { generation: 1, .. }]), "another generation's is not given back: {records:?}");
        sources.record(&stream(PAGE, 2, 0, b"V[2,2]".to_vec()), 0);
        let (records, _) = sources.restore(PAGE, Some(SUB), 0).unwrap();
        assert!(matches!(records.as_slice(), [Held::Paint { generation: 2, .. }, Held::Stream { generation: 2, .. }]), "{records:?}");
        // The sub-page's other language.
        sources.record(&paint(Language::Naplps, 2), 0);
        let (records, _) = sources.restore(PAGE, Some(SUB), 0).unwrap();
        assert!(matches!(records.as_slice(), [Held::Stream { generation: 2, .. }]), "another language's is not given back: {records:?}");
    }

    // PAR-34: past MAX_KEPT_PICTURE_SOURCE_BYTES the source drawn longest
    // ago is dropped; one larger than the whole budget is not kept.
    // Without the feature nothing is kept (REQ-1558 item 7).
    #[cfg(feature = "page-cache")]
    #[test]
    fn a_source_over_the_budget_drops_the_oldest() {
        const MEBIBYTE: usize = 1 << 20;
        let mut sources = PictureSources::default();
        let pages = u16::try_from(MAX_KEPT_PICTURE_SOURCE_BYTES / MEBIBYTE).unwrap();
        for page in 0..pages {
            sources.record(&stream(PAGE + page, 1, 0, vec![b' '; MEBIBYTE - 1024]), 0);
        }
        assert!(sources.keeps(PAGE, SUB) && sources.bytes() <= MAX_KEPT_PICTURE_SOURCE_BYTES);
        // The first is shown again: the second is now the one drawn longest ago.
        assert!(sources.restore(PAGE, Some(SUB), 0).is_some());
        sources.record(&stream(PAGE + pages, 1, 0, vec![b' '; MEBIBYTE]), 0);
        assert!(sources.bytes() <= MAX_KEPT_PICTURE_SOURCE_BYTES, "the bound");
        assert!(sources.keeps(PAGE, SUB), "drawn lately, it stays");
        assert!(!sources.keeps(PAGE + 1, SUB), "the one drawn longest ago went");
        assert!(sources.keeps(PAGE + pages, SUB));
        sources.record(&stream(0x700, 1, 0, vec![b' '; MAX_KEPT_PICTURE_SOURCE_BYTES + 1]), 0);
        assert!(!sources.keeps(0x700, SUB) && sources.keeps(PAGE, SUB), "larger than the budget: not kept, and nothing else dropped for it");
    }

    // PAR-34: the cache's lifetime applies to picture sources.
    // Without the feature nothing is kept (REQ-1558 item 7).
    #[cfg(feature = "page-cache")]
    #[test]
    fn a_picture_source_past_its_lifetime_is_not_shown_again() {
        let mut sources = PictureSources::default();
        sources.record(&stream(PAGE, 1, 0, b"P[1,1]".to_vec()), 0);
        assert!(sources.restore(PAGE, Some(SUB), CACHED_PAGE_LIFETIME_US).is_some());
        assert!(sources.restore(PAGE, Some(SUB), CACHED_PAGE_LIFETIME_US + 1).is_none());
        assert!(!sources.keeps(PAGE, SUB), "dropped");
    }
}
