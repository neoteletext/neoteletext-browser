//! The engine for one channel: it reads the channel's bytes, keeps its
//! pages, reports which have arrived or changed, and renders them.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant as Clock;

use interface_core::canvas::{PAGE_HEIGHT, PAGE_WIDTH};
use interface_core::channel::{PAGES, page_index};
use neoteletext_audio::receiver::Receiver;
use neoteletext_core::acq::{Decoder, EnhancementPacket, EnhancementStore, EnhancementSubPage, StoredPage};
use neoteletext_core::charset::{CharRef, CharSet, G2Set};
use neoteletext_core::compose::compose_level;
use neoteletext_core::config::Config;
use neoteletext_core::consts::{COLUMNS, G2_RIGHT_GUILLEMET, ROWS};
use neoteletext_core::consts::{ENHANCEMENT_PACKETS_LEVEL_35, MAGAZINES, MOT_PACKETS, PAGE_COMPLETION_TIMEOUT_US};
use neoteletext_core::fsm::PageState;
use neoteletext_core::glyphs::GlyphSet;
use neoteletext_core::level1::{DisplayOptions, Glyph, Grid, build_grid};
#[cfg(feature = "links")]
use neoteletext_core::nav::PageLinks;
use neoteletext_core::packet::{Packet, PacketMeta};
use neoteletext_core::render::{ChangeTracker, FlashPhase, IndexBuffer, IndexFormat, RenderOptions, render_screen};
use neoteletext_core::types::{Instant, PageAddress, SubCode};
use neoteletext_formats::Item;
use neoteletext_formats::detect::{AutoConfig, AutoReader, Format};
use neoteletext_formats::item::Reader;
use neoteletext_formats::t42::T42Variant;
use neoteletext_image::palette::{Rgba, colour_map_palette};
use protocol::audio::Audio;
use protocol::decoder::{FIRST_STORE_PAGES, MAX_STORE_PAGES, View};
use std::cell::RefCell;

#[cfg(feature = "interaction")]
use crate::InputRecords;
use crate::changes::Changes;
#[cfg(feature = "links")]
use crate::links;
use crate::title::title;
use crate::voice_sink::Outbox;
use crate::{Changed, FlashRates};

#[cfg(test)]
mod tests;

/// The most log lines held between two takes: a page carries at most the
/// engine's `MAX_LOG_LINES_PER_PAGE`, and a feed a few pages; past this a
/// channel that writes many is cut, not kept.
const MAX_LOGS_HELD: usize = 64;

/// A log line the channel wrote for its author.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelLog {
    /// Its level.
    pub level: interface_core::console::Level,
    /// The page it followed, as `0xMPP`.
    pub page: Option<u16>,
    /// Its text, checked by the engine.
    pub text: String,
}

/// Keeps a log line read, within [`MAX_LOGS_HELD`].
fn keep_log(logs: &mut Vec<ChannelLog>, line: &neoteletext_formats::log::LogLine<'_>, page: Option<PageAddress>) {
    use neoteletext_formats::log::LogLevel;
    if logs.len() >= MAX_LOGS_HELD {
        return;
    }
    let level = match line.level {
        LogLevel::Error => interface_core::console::Level::Error,
        LogLevel::Warning => interface_core::console::Level::Warning,
        LogLevel::Information => interface_core::console::Level::Info,
    };
    logs.push(ChannelLog { level, page: page.map(PageAddress::mpp), text: line.text.to_owned() });
}

/// Packets kept for enhancement pages: the Level 3.5 total of EN 300 706
/// Annex E.8 plus the eight Magazine Organization Tables, as the engine's
/// sessions keep.
const ENHANCEMENT_PACKETS: usize = ENHANCEMENT_PACKETS_LEVEL_35 + MAGAZINES * MOT_PACKETS;
/// Enhancement sub-pages: one for every eight packets, plus the eight tables.
const ENHANCEMENT_SUB_PAGES: usize = ENHANCEMENT_PACKETS_LEVEL_35 / 8 + MAGAZINES;
/// Microseconds between packets of a stream with no time stamps: one
/// television line, so the page timeout never fires between packets.
const LINE_PERIOD_US: u64 = 64;
/// Every row of the screen (the renderer's bitmask of rows to draw).
const ALL_ROWS: u32 = u32::MAX;
/// What shows through transparent areas: black, as on a television.
const PICTURE: Rgba = Rgba::opaque(0, 0, 0);
const PAGE_DOTS: usize = PAGE_WIDTH as usize * PAGE_HEIGHT as usize;
#[cfg(feature = "links")]
const FASTEXT_ROW: usize = 24;

static GLYPHS: GlyphSet<'static> = GlyphSet::bedstead();

/// The audio store of a desktop receiver (the engine's REQ-AUD-017, Level
/// B): 4 MB of compressed sound, 256 sounds and 8 records being reassembled.
const AUDIO_ARENA: usize = 4 * 1024 * 1024;
const AUDIO_OBJECTS: usize = 256;
const AUDIO_RECORDS: usize = 8;

/// A display page as stored: its number, its 25 rows of 7-bit codes, and
/// its title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exported {
    /// The page number.
    pub page: u16,
    /// Each row's character codes.
    pub rows: Vec<[u8; COLUMNS]>,
    /// The page's title.
    pub title: String,
}

/// A rendered page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rendered {
    /// Red, green and blue of each colour the page uses.
    pub palette: Vec<[u8; 3]>,
    /// The palette index of the screen around the page: its Full Screen
    /// Colour, else its Default Screen Colour (EN 300 706 clause 12.3,
    /// Table 4), as the engine resolved them.
    pub screen: u8,
    /// One palette index per dot, row by row.
    pub indices: Vec<u8>,
    /// The flash phase it was drawn in and the rates its cells flash at,
    /// when the page has flashing cells: it is drawn again only when the
    /// phase of a rate it uses changes.
    pub flash: Option<(FlashPhase, FlashRates)>,
    /// The links found in the page (`hyperlinks.md` section 6).
    #[cfg(feature = "links")]
    pub links: Found,
    /// The page's sub-codes in ascending order, and the one drawn (the
    /// engine's REQ-NAV-005).
    pub subpages: (Vec<u16>, u16),
}

/// The links of a page as the decoder finds them, untrusted.
#[cfg(feature = "links")]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    /// Page numbers in its text and runs its `LT` lines declare.
    pub candidates: Vec<protocol::links::Candidate>,
    /// Its `LK` lines.
    pub numbers: Vec<protocol::links::NumberLink>,
    /// Its coloured keys and Index, from packet X/27/0, and whether row 24
    /// is shown (`page-controls.md` section 2).
    pub fastext: protocol::links::Fastext,
}

/// The teletext engine for one channel: it reads the channel's bytes,
/// keeps its pages, reports which have arrived or changed, and renders them.
pub struct Engine {
    decoder: Decoder<'static>,
    reader: AutoReader,
    /// Pushed TTI pages and pushed packets, each read in its own form: a
    /// channel may fetch one and push the other (the engine's REQ-WEB-005).
    pushed_pages: AutoReader,
    pushed_packets: AutoReader,
    /// The pages the channel has pushed: a fetched copy of one is older
    /// than what the channel pushed, and is dropped.
    pushed: BTreeSet<u16>,
    /// For each magazine, whether its page now arriving in fetched data is
    /// one the channel pushed, whose packets are dropped.
    skipping: [bool; MAGAZINES],
    clock: u64,
    /// The display pages already reported as arrived, each with a digest
    /// of its content, to tell when a new version changes it.
    reported: Vec<Option<u64>>,
    started: Clock,
    /// Pages already reported whose content has since changed.
    updated: Vec<u16>,
    /// What the engine's events said changed since it was last asked.
    changes: Changes,
    /// The pages and sub-pages the store has room for.
    store_pages: usize,
    /// Every page and sub-page whose reception started: when there are as
    /// many as the store has room for, it is full.
    started_pages: BTreeSet<(u16, u16)>,
    /// The full store was reported.
    full_told: bool,
    /// The channel's log lines read since they were last taken, at most
    /// [`MAX_LOGS_HELD`] (the engine's REQ-INP-026).
    logs: Vec<ChannelLog>,
    /// Pages whose content was compared to tell whether it changed: the
    /// tests count them.
    #[cfg(test)]
    digests: usize,
    /// The channel's sounds: the audio packets go here, not to the decoder.
    audio: Receiver<'static, AUDIO_OBJECTS, AUDIO_RECORDS>,
    /// Voice operations waiting to be sent.
    sounds: Vec<Audio>,
    /// The `LT` and `LK` lines of every page file read.
    #[cfg(feature = "links")]
    lines: links::Lines,
    /// The row 24 the receiver draws for each page file with `LN` lines,
    /// by page and sub-code (REQ-NAV-011, the engine's E.108).
    receiver_rows: BTreeMap<(u16, u16), ReceiverRow>,
    /// Which rows of each page drawn changed since it was last drawn (the
    /// engine's REQ-RND-019 and REQ-RND-032): a flash phase redraws only
    /// the rows that look different in it (the engine's E.107).
    trackers: RefCell<BTreeMap<u16, ChangeTracker>>,
    /// How the pages the user changed are drawn: reveal, hold, mix and a
    /// sub-page (`page-controls.md`, decision 67).
    views: BTreeMap<u16, View>,
    /// Held pages, as they stood when held (the engine's REQ-NAV-006).
    held: BTreeMap<u16, Box<StoredPage>>,
    /// The channel's records for its input found since the last call, as
    /// sent, for the trusted core to check (INPUT.md section 7).
    #[cfg(feature = "interaction")]
    input: InputRecords,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// An engine with an empty page store of the first size.
    #[must_use]
    pub fn new() -> Self {
        Self::with_store(FIRST_STORE_PAGES)
    }

    /// An engine with an empty page store of `store_pages` pages and
    /// sub-pages, at most [`MAX_STORE_PAGES`].
    #[must_use]
    pub fn with_store(store_pages: usize) -> Self {
        let store_pages = store_pages.clamp(1, MAX_STORE_PAGES);
        // One store for the life of the process, as the engine's sessions
        // do; a bigger one needs a new process (decision 47).
        let slots: &'static mut [StoredPage] = Box::leak(vec![StoredPage::EMPTY; store_pages].into_boxed_slice());
        let sub_pages = Box::leak(vec![EnhancementSubPage::EMPTY; ENHANCEMENT_SUB_PAGES].into_boxed_slice());
        let packets = Box::leak(vec![EnhancementPacket::EMPTY; ENHANCEMENT_PACKETS].into_boxed_slice());
        let decoder = Decoder::with_enhancement_store(slots, EnhancementStore::new(sub_pages, packets), Config::default());
        // Detection decides the format; page files are encoded into
        // packets, so they take the same path as broadcasts.
        let reader = AutoReader::new(AutoConfig { encode_pages: true, ..AutoConfig::default() });
        let pushed_pages = AutoReader::new(AutoConfig { format: Some(Format::Tti), encode_pages: true, ..AutoConfig::default() });
        let pushed_packets = AutoReader::new(AutoConfig { format: Some(Format::T42(T42Variant::Plain)), ..AutoConfig::default() });
        let arena: &'static mut [u8] = Box::leak(vec![0_u8; AUDIO_ARENA].into_boxed_slice());
        Self {
            decoder,
            reader,
            pushed_pages,
            pushed_packets,
            pushed: BTreeSet::new(),
            skipping: [false; MAGAZINES],
            clock: 0,
            reported: vec![None; PAGES],
            started: Clock::now(),
            updated: Vec::new(),
            changes: Changes::default(),
            store_pages,
            started_pages: BTreeSet::new(),
            full_told: false,
            logs: Vec::new(),
            #[cfg(test)]
            digests: 0,
            audio: Receiver::new(arena),
            sounds: Vec::new(),
            #[cfg(feature = "links")]
            lines: links::Lines::default(),
            receiver_rows: BTreeMap::new(),
            trackers: RefCell::new(BTreeMap::new()),
            views: BTreeMap::new(),
            held: BTreeMap::new(),
            #[cfg(feature = "interaction")]
            input: InputRecords::default(),
        }
    }

    /// How `page` is drawn from now on: a page held keeps the version it
    /// shows now (`page-controls.md` section 3, decision 67).
    pub fn set_view(&mut self, view: View) {
        let page = view.page;
        if !view.hold {
            self.held.remove(&page);
        } else if !self.held.contains_key(&page) || self.views.get(&page).map(|before| before.subpage) != Some(view.subpage) {
            let showing = self.stored(page, view.subpage).copied();
            if let Some(stored) = showing {
                self.held.insert(page, Box::new(stored));
            }
        }
        if view == (View { page, ..View::default() }) {
            self.views.remove(&page);
        } else {
            self.views.insert(page, view);
        }
    }

    /// The stored sub-page of `page` to draw: the one chosen, while the
    /// store has it, else the latest.
    fn stored(&self, page: u16, subpage: Option<u16>) -> Option<&StoredPage> {
        let address = PageAddress::from_mpp(page)?;
        let store = self.decoder.store();
        subpage.and_then(|subpage| store.get(address, SubCode::new(subpage))).or_else(|| store.get(address, SubCode::ANY))
    }

    /// The sub-codes stored for `page`, in ascending order, at most
    /// [`interface_core::controls::MAX_SUBPAGES`].
    fn subcodes(&self, page: u16) -> Vec<u16> {
        let Some(address) = PageAddress::from_mpp(page) else { return Vec::new() };
        let mut codes = [SubCode::ANY; neoteletext_core::viewer::MAX_LISTED_SUBPAGES];
        let count = self.decoder.store().subcodes(address, &mut codes);
        codes.iter().take(count).map(|code| code.value()).collect()
    }

    /// The voice operations produced since the last call, once each.
    pub fn take_sounds(&mut self) -> Vec<Audio> {
        core::mem::take(&mut self.sounds)
    }

    /// The user opened `page`, or none: its sounds play while it is open
    /// (the engine's REQ-AUD-012).
    pub fn open_page(&mut self, page: Option<u16>) {
        self.audio.open_page(page, &mut Outbox(&mut self.sounds));
    }

    /// The time, in milliseconds from any start: a live sound with no new
    /// block for two seconds fades out (the engine's REQ-AUD-014). Called
    /// before each piece of data and at each tick.
    pub fn tick_audio(&mut self, now_ms: u64) {
        self.audio.tick(now_ms, &mut Outbox(&mut self.sounds));
    }

    /// The channel's records for its input found since the last call, once
    /// each.
    #[cfg(feature = "interaction")]
    pub fn take_input(&mut self) -> InputRecords {
        core::mem::take(&mut self.input)
    }

    fn push_all(&mut self, packets: &[(Packet, PacketMeta)]) {
        for (packet, meta) in packets {
            // The channel's cursor and control records (INPUT.md section 7)
            // ride the audio profile's data channel; only one that reads
            // goes on, as its body.
            #[cfg(feature = "interaction")]
            if neoteletext_input::is_input_packet(packet) {
                self.input.take_packet(packet);
                continue;
            }
            // The audio profile's packets go to its receiver; the decoder
            // ignores packet 31 anyway (the engine's REQ-PKT-008).
            if self.audio.push_packet(packet, &mut Outbox(&mut self.sounds)) {
                continue;
            }
            self.clock = match meta.timestamp_us {
                Some(stamp) if stamp >= self.clock => stamp,
                _ => self.clock.saturating_add(LINE_PERIOD_US),
            };
            let changes = &mut self.changes;
            self.decoder.push(packet, meta, Instant(self.clock), &mut |event| changes.note(&event));
        }
    }

    /// Feeds more of the channel's bytes; returns the pages that arrived.
    /// Pages that changed are kept for [`Engine::take_updated`].
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<(u16, String)> {
        let mut packets = Vec::new();
        let mut pages = PageLines {
            #[cfg(feature = "links")]
            lines: &mut self.lines,
            #[cfg(feature = "interaction")]
            input: &mut self.input,
            receiver_rows: &mut self.receiver_rows,
            unused: core::marker::PhantomData,
        };
        let logs = &mut self.logs;
        self.reader.feed(bytes, &mut |item| match item {
            Item::Packet { packet, meta } => packets.push((packet, meta)),
            // A page file's link and cursor lines are kept beside its packets.
            Item::Page(page) => pages.read(page),
            Item::Log { line, page, .. } => keep_log(logs, &line, page),
            _ => {}
        });
        self.drop_pushed(&mut packets);
        self.push_all(&packets);
        self.arrivals()
    }

    /// Feeds one whole page file, bundle or pushed message, and completes
    /// the pages in it at once: a page shows as it arrives, not when the
    /// next page of its magazine begins (`interaction.md` section 10). A
    /// fetched copy of a page the channel pushed is older, and dropped.
    pub fn feed_whole(&mut self, packets: bool, pushed: bool, bytes: &[u8]) -> Vec<(u16, String)> {
        let mut items = Vec::new();
        let mut pages = PageLines {
            #[cfg(feature = "links")]
            lines: &mut self.lines,
            #[cfg(feature = "interaction")]
            input: &mut self.input,
            receiver_rows: &mut self.receiver_rows,
            unused: core::marker::PhantomData,
        };
        let reader = if packets { &mut self.pushed_packets } else { &mut self.pushed_pages };
        let logs = &mut self.logs;
        let mut take = |item: Item<'_>| match item {
            Item::Packet { packet, meta } => items.push((packet, meta)),
            Item::Page(page) => pages.read(page),
            Item::Log { line, page, .. } => keep_log(logs, &line, page),
            _ => {}
        };
        reader.feed(bytes, &mut take);
        reader.finish(&mut take);
        if pushed {
            for (packet, _) in &items {
                if let Some(page) = header_page(packet) {
                    self.pushed.insert(page);
                }
            }
        } else {
            self.drop_pushed(&mut items);
        }
        self.push_all(&items);
        self.complete();
        self.arrivals()
    }

    /// Drops the fetched packets of pages the channel has pushed: a page
    /// file fetched in the contents' order may arrive after the channel
    /// pushed a newer version, and must not replace it.
    fn drop_pushed(&mut self, packets: &mut Vec<(Packet, PacketMeta)>) {
        if self.pushed.is_empty() {
            return;
        }
        let (pushed, skipping) = (&self.pushed, &mut self.skipping);
        packets.retain(|(packet, _)| {
            let Ok(address) = neoteletext_core::packet::decode_address(packet) else { return true };
            let magazine = usize::from(address.magazine.number()) % MAGAZINES;
            let Some(skip) = skipping.get_mut(magazine) else { return true };
            if address.number == 0 {
                *skip = header_page(packet).is_some_and(|page| pushed.contains(&page));
            }
            !*skip
        });
    }

    /// Completes every page still being received, as the end of a stream
    /// or a timeout does (EN 300 706 clause 7.2: a page ends with the next
    /// header of its magazine, or when the receiver gives up waiting).
    fn complete(&mut self) {
        self.clock = self.clock.saturating_add(PAGE_COMPLETION_TIMEOUT_US.saturating_mul(2));
        let changes = &mut self.changes;
        self.decoder.tick(Instant(self.clock), &mut |event| changes.note(&event));
    }

    /// The pages whose packets arrived, or whose reception started, ended
    /// or was erased, since the last call; and whether data every page may
    /// use changed (a magazine's or an enhancement page's), when every
    /// shown page is to be drawn again.
    pub fn take_changed(&mut self) -> Changed {
        let changes = core::mem::take(&mut self.changes);
        self.started_pages.extend(changes.started.iter().copied());
        // Arrivals are still to be worked out from the completed pages.
        self.changes.completed = changes.completed;
        Changed { pages: changes.touched, everything: changes.everything }
    }

    /// The channel's log lines read since the last call.
    pub fn take_logs(&mut self) -> Vec<ChannelLog> {
        core::mem::take(&mut self.logs)
    }

    /// True once, when as many pages and sub-pages have been received as
    /// the store has room for: the oldest are being forgotten, and the
    /// decoder should be started again with more room.
    pub fn take_store_full(&mut self) -> bool {
        if self.full_told || self.started_pages.len() < self.store_pages {
            return false;
        }
        self.full_told = true;
        true
    }

    /// Microseconds since the engine started, on the clock it renders flash
    /// and keeps sounds by.
    #[must_use]
    pub fn now_us(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_micros()).unwrap_or(u64::MAX)
    }

    /// When the audio receiver next needs [`Engine::tick_audio`], in
    /// milliseconds on [`Engine::now_us`]'s clock: `None` while no live
    /// sound plays (the engine's decision E.98).
    #[must_use]
    pub fn next_audio_tick_ms(&self) -> Option<u64> {
        self.audio.next_tick_ms()
    }

    /// The pages whose content changed since they were reported, once each.
    pub fn take_updated(&mut self) -> Vec<u16> {
        core::mem::take(&mut self.updated)
    }

    /// Ends the channel's data: completes every page still being received.
    pub fn finish(&mut self) -> Vec<(u16, String)> {
        let mut packets = Vec::new();
        let mut pages = PageLines {
            #[cfg(feature = "links")]
            lines: &mut self.lines,
            #[cfg(feature = "interaction")]
            input: &mut self.input,
            receiver_rows: &mut self.receiver_rows,
            unused: core::marker::PhantomData,
        };
        let logs = &mut self.logs;
        self.reader.finish(&mut |item| match item {
            Item::Packet { packet, meta } => packets.push((packet, meta)),
            Item::Page(page) => pages.read(page),
            Item::Log { line, page, .. } => keep_log(logs, &line, page),
            _ => {}
        });
        self.drop_pushed(&mut packets);
        self.push_all(&packets);
        self.complete();
        self.arrivals()
    }

    /// Display pages received completely and not yet reported, each with
    /// a title from its text.
    fn arrivals(&mut self) -> Vec<(u16, String)> {
        let mut fresh = Vec::new();
        // Only pages whose reception completed can have arrived or changed:
        // the engine's events say which, so the store is not searched.
        for number in core::mem::take(&mut self.changes.completed) {
            let Some(index) = page_index(number) else { continue };
            let Some(address) = PageAddress::from_mpp(number) else { continue };
            let Some(page) = self.decoder.store().get(address, SubCode::ANY) else { continue };
            if page.state != PageState::Complete {
                continue;
            }
            #[cfg(test)]
            {
                self.digests = self.digests.saturating_add(1);
            }
            let digest = digest(page);
            let Some(reported) = self.reported.get_mut(index) else { continue };
            match *reported {
                None => fresh.push(number),
                Some(before) if before != digest => self.updated.push(number),
                Some(_) => {}
            }
            *reported = Some(digest);
        }
        self.updated.sort_unstable();
        self.updated.dedup();
        fresh.sort_unstable();
        fresh.into_iter().map(|page| (page, self.grid(page).map(|(grid, _)| title(&grid)).unwrap_or_default())).collect()
    }

    /// Every display page received completely, each with its 25 rows of
    /// stored 7-bit codes and its title: for tools that rebuild channels
    /// from recordings.
    #[must_use]
    pub fn export(&self) -> Vec<Exported> {
        let mut pages: Vec<Exported> = self
            .decoder
            .store()
            .pages()
            .iter()
            .filter(|page| page.state == PageState::Complete && page_index(page.key.address.mpp()).is_some())
            .map(|page| {
                let number = page.key.address.mpp();
                let rows = page.cells.iter().take(ROWS).copied().collect();
                Exported { page: number, rows, title: self.grid(number).map(|(grid, _)| title(&grid)).unwrap_or_default() }
            })
            .collect();
        pages.sort_by_key(|page| page.page);
        pages.dedup_by_key(|page| page.page);
        pages
    }

    /// The page's grid, as the engine composes it for display in its view
    /// (`page-controls.md`), and the stored sub-page it shows.
    fn grid(&self, page: u16) -> Option<(Grid, &StoredPage)> {
        let address = PageAddress::from_mpp(page)?;
        let view = self.views.get(&page).copied().unwrap_or_default();
        let stored = match self.held.get(&page) {
            Some(held) => held,
            None => self.stored(page, view.subpage)?,
        };
        let config = self.decoder.config();
        let magazine = self.decoder.magazine_data(address.magazine);
        let receiver = self.receiver_rows.get(&(page, stored.key.subcode.value()));
        let options = DisplayOptions { mix: view.mix, row24: receiver.map(|row| row.codes), ..DisplayOptions::default() };
        let mut grid = Grid::EMPTY;
        build_grid(&mut grid, Some(stored), &magazine, config, &options);
        let lookup = self.decoder.enhancement_lookup();
        let _ = compose_level(&mut grid, stored, &magazine, &lookup, config, &options);
        // The "»" marks, as the engine's viewer places them: Latin G2's
        // right guillemet where the codes hold `>`.
        for &column in receiver.map_or(&[][..], |row| row.marks.as_slice()) {
            if let Some(cell) = grid.cell_mut(ROWS.saturating_sub(1), column) {
                cell.glyph = Glyph::Char(CharRef::new(CharSet::G2(G2Set::Latin), G2_RIGHT_GUILLEMET));
            }
        }
        Some((grid, stored))
    }

    /// Renders the latest sub-page of `page` at the engine's flash phase
    /// for now, in the resolved colour mode, whose palette is the page's
    /// Colour Map of at most 32 colours. The page's own underline is not
    /// drawn: in NeoTeletext an underline means a link, and the browser
    /// draws links itself (decision 42, the engine's decision E.88).
    #[must_use]
    pub fn render(&self, page: u16) -> Option<Rendered> {
        self.render_at(page, self.now_us())
    }

    /// Renders `page` at the flash phase of `now_us`, on
    /// [`Engine::now_us`]'s clock.
    #[must_use]
    pub fn render_at(&self, page: u16, now_us: u64) -> Option<Rendered> {
        self.render_into(page, now_us, Vec::new())
    }

    /// [`Engine::render_at`] into `indices`, a page's worth of room kept
    /// from an earlier frame: a flashing page is rendered seven times a
    /// second, and need not take 240 KB afresh each time.
    #[must_use]
    pub fn render_into(&self, page: u16, now_us: u64, indices: Vec<u8>) -> Option<Rendered> {
        self.render_over(page, now_us, None, indices)
    }

    /// [`Engine::render_into`], drawn over `last`, the page's last frame,
    /// when there is one: only the rows that changed since it are drawn
    /// again (the engine's E.107). The page's change tracker follows every
    /// frame drawn, so `last` must be the last frame drawn of it, or one
    /// the same.
    #[must_use]
    pub fn render_over(&self, page: u16, now_us: u64, last: Option<&[u8]>, mut indices: Vec<u8>) -> Option<Rendered> {
        page_index(page)?;
        let (grid, stored) = self.grid(page)?;
        let subcode = stored.key.subcode.value();
        let reveal = self.views.get(&page).is_some_and(|view| view.reveal);
        let config = self.decoder.config();
        let phase = FlashPhase::at(Instant(now_us));
        let render = RenderOptions::resolved(config, reveal, phase).with_underline(false);
        let changed = self.trackers.borrow_mut().entry(page).or_default().changed_rows(&grid, render);
        indices.clear();
        let rows = if let Some(last) = last.filter(|last| last.len() == PAGE_DOTS) {
            indices.extend_from_slice(last);
            changed
        } else {
            indices.resize(PAGE_DOTS, 0);
            ALL_ROWS
        };
        let mut buffer = IndexBuffer { format: IndexFormat::Bits8, data: &mut indices, stride: usize::from(PAGE_WIDTH) };
        render_screen(&grid, rows, &GLYPHS, render, &mut buffer).ok()?;
        let palette: Vec<[u8; 3]> = colour_map_palette(&grid.colour_map, PICTURE).iter().map(|colour| [colour.r, colour.g, colour.b]).collect();
        #[cfg(feature = "links")]
        let links = {
            let (candidates, numbers) = links::find(&grid, &palette, self.lines.get(page, subcode));
            Found { candidates, numbers, fastext: fastext(stored, &grid) }
        };
        let rates = FlashRates::of(grid.cells.iter().flatten().filter(|cell| cell.flash).map(|cell| cell.attributes.flash_rate()));
        Some(Rendered {
            palette,
            screen: grid.screen_colour,
            indices,
            flash: (!rates.is_empty()).then_some((phase, rates)),
            #[cfg(feature = "links")]
            links,
            subpages: (self.subcodes(page), subcode),
        })
    }
}

/// A page's Fastext from its packet X/27/0, which the engine writes from a
/// TTI page's link lines too (the engine's REQ-NAV-003), and whether its
/// row 24 is shown as the engine's viewer decides it: drawn, and not boxed
/// away (REQ-SEC-024).
#[cfg(feature = "links")]
fn fastext(stored: &StoredPage, grid: &Grid) -> protocol::links::Fastext {
    let links = PageLinks::from_page(stored);
    let target = |link: Option<neoteletext_core::nav::LinkTarget>| {
        let link = link?;
        let page = link.address.mpp();
        page_index(page)?;
        let subcode = (!link.subcode.is_any()).then(|| link.subcode.value());
        Some(protocol::links::FastextTarget { page, subcode, service: link.service })
    };
    let [red, green, yellow, cyan] = links.keys;
    let row24 = !grid.boxed_mode && grid.cells.get(FASTEXT_ROW).is_some_and(|row| row.iter().any(|cell| !cell.glyph.is_blank()));
    protocol::links::Fastext { keys: [target(red), target(green), target(yellow), target(cyan), target(links.index)], row24 }
}

/// The page number a header packet starts, as teletext writes it.
fn header_page(packet: &Packet) -> Option<u16> {
    match neoteletext_core::packet::decode(packet) {
        Ok(neoteletext_core::packet::Decoded::Header(header)) => Some(PageAddress { magazine: header.magazine, page: header.page? }.mpp()),
        _ => None,
    }
}

/// What a page file says beyond its packets: its link lines and its
/// cursor line, kept as each page is read.
/// A row 24 drawn by the receiver from a page's `LN` lines: its codes, and
/// the columns where "»" marks a link that leaves the service.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ReceiverRow {
    codes: [u8; COLUMNS],
    marks: Vec<usize>,
}

struct PageLines<'a> {
    #[cfg(feature = "links")]
    lines: &'a mut links::Lines,
    receiver_rows: &'a mut BTreeMap<(u16, u16), ReceiverRow>,
    #[cfg(feature = "interaction")]
    input: &'a mut InputRecords,
    unused: core::marker::PhantomData<&'a ()>,
}

impl PageLines<'_> {
    #[cfg_attr(not(any(feature = "links", feature = "interaction")), allow(unused_variables, clippy::unused_self, reason = "the lines are compiled out"))]
    fn read(&mut self, page: &neoteletext_formats::page::PageFile) {
        #[cfg(feature = "links")]
        self.lines.read(page);
        // The engine's receiver row: the browser follows no linked service
        // (decision E.88), so a link to another service is null.
        let key = (page.address.mpp(), page.subcode.value());
        match neoteletext_formats::links::receiver_links(page, &[] as &[&str], false) {
            Some(receiver) => {
                let marks = receiver.row.marks.as_slice().iter().map(|&column| usize::from(column)).collect();
                self.receiver_rows.insert(key, ReceiverRow { codes: receiver.row.codes, marks });
            }
            None => {
                self.receiver_rows.remove(&key);
            }
        }
        // REQ-INT-006: a `CU` line is the page's cursor record.
        #[cfg(feature = "interaction")]
        if let Some(line) = page.cursor {
            let [p0, p1] = page.address.mpp().to_be_bytes();
            let [a0, a1] = line.acked.to_be_bytes();
            self.input.cursors.push([p0, p1, a0, a1, line.row, line.column, line.width, line.flags]);
        }
        // REQ-INT-114: an `IM` line is the page's own modes, as a
        // PAGE_MODES record of the channel's would say them.
        #[cfg(feature = "interaction")]
        if let Some(line) = page.input_modes {
            self.input.page_modes(page.address.mpp(), line.modes);
        }
    }
}

/// A digest of a page's rows: two versions with the same digest show the
/// same text and mosaics.
fn digest(page: &StoredPage) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    page.cells.hash(&mut hasher);
    hasher.finish()
}
