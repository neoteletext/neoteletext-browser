//! The teletext engine as the decoder process uses it. Bytes of any format
//! the engine reads (T42 recordings, TTI page files, transport streams,
//! sliced records) go through its format detection and readers into its
//! decoder and page store; pages are composed at the presentation level of
//! the engine's default profile and rendered by its row renderer with its
//! bundled font, Bedstead, into palette indices.
//!
//! Everything here reads untrusted bytes, which is why it runs in a process
//! of its own with no network and no files (design section 5).

// No panicking constructs: this crate reads input from outside the
// process. The same list is in each such crate; scripts/check.sh checks
// that they agree.
#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant as Clock;

#[cfg(feature = "links")]
pub mod links;

use interface_core::canvas::{PAGE_HEIGHT, PAGE_WIDTH};
use interface_core::channel::{PAGES, page_index};
use neoteletext_audio::port::{SoundFormat, SoundId, Start, VoiceSink};
use neoteletext_audio::receiver::Receiver;
use neoteletext_core::acq::{Decoder, EnhancementPacket, EnhancementStore, EnhancementSubPage, Event, StoredPage};
use neoteletext_core::charset::to_unicode;
use neoteletext_core::compose::compose_level;
use neoteletext_core::config::Config;
use neoteletext_core::consts::{COLUMNS, FLASH_CYCLE_US, FLASH_UNITS, ROWS};
use neoteletext_core::consts::{ENHANCEMENT_PACKETS_LEVEL_35, MAGAZINES, MOT_PACKETS, PAGE_COMPLETION_TIMEOUT_US};
use neoteletext_core::fsm::PageState;
use neoteletext_core::glyphs::GlyphSet;
use neoteletext_core::level1::{DisplayOptions, FlashRate, Glyph, Grid, build_grid};
use neoteletext_core::packet::{Packet, PacketMeta};
use neoteletext_core::render::{FlashPhase, IndexBuffer, IndexFormat, RenderOptions, render_screen};
use neoteletext_core::types::{Instant, PageAddress, SubCode};
use neoteletext_formats::Item;
use neoteletext_formats::detect::{AutoConfig, AutoReader, Format};
use neoteletext_formats::item::Reader;
use neoteletext_formats::t42::T42Variant;
use neoteletext_image::palette::{Rgba, colour_map_palette};
use protocol::audio::{Audio, MAX_MESSAGE_SAMPLES, Sound};
use protocol::decoder::{FIRST_STORE_PAGES, MAX_STORE_PAGES};

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
/// A title is the first text row with at least this many letters or digits.
const TITLE_MINIMUM: usize = 3;
/// A row is a picture, not a title, when more than a quarter of its drawn
/// cells are mosaics.
const MOSAIC_SHARE: usize = 3;
/// Row 0 is the header; text rows are 1 to 23, row 24 the Fastext row.
const FIRST_TEXT_ROW: usize = 1;
const LAST_TEXT_ROW: usize = 23;

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
    /// One palette index per dot, row by row.
    pub indices: Vec<u8>,
    /// The flash phase it was drawn in and the rates its cells flash at,
    /// when the page has flashing cells: it is drawn again only when the
    /// phase of a rate it uses changes.
    pub flash: Option<(FlashPhase, FlashRates)>,
    /// The links found in the page (`hyperlinks.md` section 6).
    #[cfg(feature = "links")]
    pub links: Found,
}

/// The links of a page as the decoder finds them, untrusted.
#[cfg(feature = "links")]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    /// Page numbers in its text and runs its `LT` lines declare.
    pub candidates: Vec<protocol::links::Candidate>,
    /// Its `LK` lines.
    pub numbers: Vec<protocol::links::NumberLink>,
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
    /// The channel's records for its input found since the last call, as
    /// sent, for the trusted core to check (INPUT.md section 7).
    #[cfg(feature = "interaction")]
    input: InputRecords,
}

/// The channel's records for its input, untrusted, in their order of
/// arrival within each kind.
#[cfg(feature = "interaction")]
#[derive(Debug, Default, PartialEq, Eq)]
pub struct InputRecords {
    /// Cursor records' bodies (REQ-INT-005, REQ-INT-110), from packets and
    /// `CU` lines.
    pub cursors: Vec<[u8; neoteletext_input::cursor::BODY]>,
    /// Control messages' bodies (REQ-INT-111), from packets, and a page's
    /// `IM` line as `PAGE_MODES` (REQ-INT-114).
    pub controls: Vec<Vec<u8>>,
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
            #[cfg(test)]
            digests: 0,
            audio: Receiver::new(arena),
            sounds: Vec::new(),
            #[cfg(feature = "links")]
            lines: links::Lines::default(),
            #[cfg(feature = "interaction")]
            input: InputRecords::default(),
        }
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
            unused: core::marker::PhantomData,
        };
        self.reader.feed(bytes, &mut |item| match item {
            Item::Packet { packet, meta } => packets.push((packet, meta)),
            // A page file's link and cursor lines are kept beside its packets.
            Item::Page(page) => pages.read(page),
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
            unused: core::marker::PhantomData,
        };
        let reader = if packets { &mut self.pushed_packets } else { &mut self.pushed_pages };
        let mut take = |item: Item<'_>| match item {
            Item::Packet { packet, meta } => items.push((packet, meta)),
            Item::Page(page) => pages.read(page),
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
            unused: core::marker::PhantomData,
        };
        self.reader.finish(&mut |item| match item {
            Item::Packet { packet, meta } => packets.push((packet, meta)),
            Item::Page(page) => pages.read(page),
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

    /// The page's grid, as the engine composes it for display, and the
    /// sub-code of the sub-page shown.
    fn grid(&self, page: u16) -> Option<(Grid, u16)> {
        let address = PageAddress::from_mpp(page)?;
        let stored = self.decoder.store().get(address, SubCode::ANY)?;
        let config = self.decoder.config();
        let magazine = self.decoder.magazine_data(address.magazine);
        let options = DisplayOptions::default();
        let mut grid = Grid::EMPTY;
        build_grid(&mut grid, Some(stored), &magazine, config, &options);
        let lookup = self.decoder.enhancement_lookup();
        let _ = compose_level(&mut grid, stored, &magazine, &lookup, config, &options);
        Some((grid, stored.key.subcode.value()))
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
    pub fn render_into(&self, page: u16, now_us: u64, mut indices: Vec<u8>) -> Option<Rendered> {
        page_index(page)?;
        #[cfg_attr(not(feature = "links"), allow(unused_variables, reason = "the sub-code finds a page's link lines"))]
        let (grid, subcode) = self.grid(page)?;
        let config = self.decoder.config();
        let phase = FlashPhase::at(Instant(now_us));
        let render = RenderOptions::resolved(config, false, phase).with_underline(false);
        indices.clear();
        indices.resize(PAGE_DOTS, 0);
        let mut buffer = IndexBuffer { format: IndexFormat::Bits8, data: &mut indices, stride: usize::from(PAGE_WIDTH) };
        render_screen(&grid, ALL_ROWS, &GLYPHS, render, &mut buffer).ok()?;
        let palette: Vec<[u8; 3]> = colour_map_palette(&grid.colour_map, PICTURE).iter().map(|colour| [colour.r, colour.g, colour.b]).collect();
        #[cfg(feature = "links")]
        let links = {
            let (candidates, numbers) = links::find(&grid, &palette, self.lines.get(page, subcode));
            Found { candidates, numbers }
        };
        let rates = FlashRates::of(grid.cells.iter().flatten().filter(|cell| cell.flash).map(|cell| cell.attributes.flash_rate()));
        Some(Rendered {
            palette,
            indices,
            flash: (!rates.is_empty()).then_some((phase, rates)),
            #[cfg(feature = "links")]
            links,
        })
    }
}

/// What the engine's events said changed.
#[derive(Default)]
struct Changes {
    touched: BTreeSet<u16>,
    completed: BTreeSet<u16>,
    /// Pages and sub-pages whose reception started.
    started: BTreeSet<(u16, u16)>,
    everything: bool,
}

impl Changes {
    fn note(&mut self, event: &Event) {
        match event {
            Event::PageStarted(key) => {
                self.touched.insert(key.address.mpp());
                self.started.insert((key.address.mpp(), key.subcode.value()));
            }
            Event::PageErased(key) | Event::UpdateIndicator(key) | Event::PageUpdated { key, .. } => {
                self.touched.insert(key.address.mpp());
            }
            Event::PageCompleted(key) => {
                self.touched.insert(key.address.mpp());
                self.completed.insert(key.address.mpp());
            }
            // Data every page of a magazine, or any page, may draw from.
            Event::MagazineData(_) | Event::EnhancementPageChanged(_) => self.everything = true,
            _ => {}
        }
    }
}

/// What changed since [`Engine::take_changed`] was last called.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changed {
    /// The display pages whose reception moved.
    pub pages: BTreeSet<u16>,
    /// Every page may draw differently.
    pub everything: bool,
}

/// The flash rates of the engine's decision E.48, in the order of
/// [`FlashRates`]' bits.
const RATES: [FlashRate; 4] = [FlashRate::Slow, FlashRate::FastPhase1, FlashRate::FastPhase2, FlashRate::FastPhase3];

/// The set of flash rates a page's cells use: one bit for each of
/// [`RATES`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlashRates(u8);

impl FlashRates {
    /// The rates of `cells`.
    pub fn of(cells: impl Iterator<Item = FlashRate>) -> Self {
        Self(cells.fold(0, |used, rate| used | RATES.iter().position(|&each| each == rate).map_or(0, |index| 1_u8 << index)))
    }

    fn is_empty(self) -> bool {
        self.0 == 0
    }

    fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether any of these rates is in a different phase in `drawn` and
    /// `now`: whether a page drawn in `drawn` looks different at `now`.
    fn differ(self, drawn: FlashPhase, now: FlashPhase) -> bool {
        RATES.iter().enumerate().any(|(index, &rate)| self.0 & (1_u8 << index) != 0 && drawn.is_on(rate) != now.is_on(rate))
    }
}

/// The shown pages with flashing cells, each with the flash phase it was
/// last drawn in and the rates it uses: the decoder wakes for flash only
/// when one of them is shown, and only when a rate one of them uses
/// changes phase, and draws a page again only when a rate it uses did
/// (requirement 9.7). A page with slow flash alone is drawn twice a
/// second, not seven times.
#[derive(Debug, Default)]
pub struct Flashing {
    pages: BTreeMap<u16, (FlashPhase, FlashRates)>,
}

impl Flashing {
    /// Records how `page` was last drawn.
    pub fn drawn(&mut self, page: u16, rendered: &Rendered) {
        match rendered.flash {
            Some(flash) => self.pages.insert(page, flash),
            None => self.pages.remove(&page),
        };
    }

    /// Keeps only the pages still shown.
    pub fn retain(&mut self, shown: &[u16]) {
        self.pages.retain(|page, _| shown.contains(page));
    }

    /// The flashing pages that look different at `now_us` from how they
    /// were drawn.
    #[must_use]
    pub fn due(&self, now_us: u64) -> Vec<u16> {
        let phase = FlashPhase::at(Instant(now_us));
        self.pages.iter().filter(|&(_, &(drawn, rates))| rates.differ(drawn, phase)).map(|(&page, _)| page).collect()
    }

    /// When a rate some shown page uses next changes phase, if any shown
    /// page flashes: the start of that one of the twelve units of the
    /// one-second cycle (the engine's decision E.48).
    #[must_use]
    pub fn next_us(&self, now_us: u64) -> Option<u64> {
        let used = self.pages.values().fold(FlashRates::default(), |used, &(_, rates)| used.union(rates));
        if used.is_empty() {
            return None;
        }
        let units = u64::from(FLASH_UNITS);
        let cycle = now_us.checked_div(FLASH_CYCLE_US).unwrap_or(0).saturating_mul(FLASH_CYCLE_US);
        let within = now_us.saturating_sub(cycle);
        let unit = within.saturating_mul(units).checked_div(FLASH_CYCLE_US).unwrap_or(0);
        let phase = FlashPhase::at_unit(u8::try_from(unit).unwrap_or(0));
        // The first later unit whose phase differs for a rate in use; every
        // rate changes within a cycle, so one is found.
        let ahead = (1..=units).find(|&ahead| used.differ(phase, FlashPhase::at_unit(u8::try_from(unit.saturating_add(ahead)).unwrap_or(0)))).unwrap_or(1);
        let next = unit.saturating_add(ahead).saturating_mul(FLASH_CYCLE_US).div_ceil(units);
        Some(cycle.saturating_add(next))
    }
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
struct PageLines<'a> {
    #[cfg(feature = "links")]
    lines: &'a mut links::Lines,
    #[cfg(feature = "interaction")]
    input: &'a mut InputRecords,
    unused: core::marker::PhantomData<&'a ()>,
}

impl PageLines<'_> {
    #[cfg_attr(not(any(feature = "links", feature = "interaction")), allow(unused_variables, clippy::unused_self, reason = "the lines are compiled out"))]
    fn read(&mut self, page: &neoteletext_formats::page::PageFile) {
        #[cfg(feature = "links")]
        self.lines.read(page);
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

/// The voice sink of a decoder: each operation becomes a message for the
/// trusted core.
struct Outbox<'a>(&'a mut Vec<Audio>);

fn sound(id: SoundId) -> Sound {
    Sound { handle: id.handle, generation: id.generation }
}

impl VoiceSink for Outbox<'_> {
    fn define(&mut self, id: SoundId, format: SoundFormat) {
        self.0.push(Audio::Define { sound: sound(id), rate_hz: format.rate_hz, samples: format.samples });
    }

    fn samples(&mut self, id: SoundId, first: u32, pcm: &[i16]) {
        let mut start = first;
        for piece in pcm.chunks(MAX_MESSAGE_SAMPLES) {
            self.0.push(Audio::Samples { sound: sound(id), first: start, pcm: piece.to_vec() });
            start = start.saturating_add(u32::try_from(piece.len()).unwrap_or(u32::MAX));
        }
    }

    fn forget(&mut self, id: SoundId) {
        self.0.push(Audio::Forget(sound(id)));
    }

    fn stream(&mut self, id: SoundId, rate_hz: u32) {
        self.0.push(Audio::Stream { sound: sound(id), rate_hz });
    }

    fn stream_samples(&mut self, id: SoundId, pcm: &[i16]) {
        for piece in pcm.chunks(MAX_MESSAGE_SAMPLES) {
            self.0.push(Audio::StreamSamples { sound: sound(id), pcm: piece.to_vec() });
        }
    }

    fn start(&mut self, channel: u8, start: Start) {
        self.0.push(Audio::Start {
            channel,
            sound: sound(start.sound),
            gain: start.gain,
            fade_ms: start.fade_ms,
            looped: start.looped,
            duck_percent: start.duck_percent,
            after_current: start.after_current,
        });
    }

    fn stop(&mut self, channel: u8, fade_ms: u16) {
        self.0.push(Audio::Stop { channel, fade_ms });
    }

    fn gain(&mut self, channel: u8, gain: u8, fade_ms: u16) {
        self.0.push(Audio::Gain { channel, gain, fade_ms });
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

/// The first text row with a few letters or digits: a title for the page
/// list. The engine's grid says which cells are text, as opposed to
/// mosaics, and its character sets give each text cell's character, with
/// the page's national characters. Concealed text is left out.
fn title(grid: &Grid) -> String {
    for row in FIRST_TEXT_ROW..=LAST_TEXT_ROW {
        // A row drawn mostly in mosaics is a picture: the capitals in it
        // are graphics too, shown as letters by the blast-through rule.
        let glyphs = (0..COLUMNS).filter_map(|column| grid.cell(row, column).map(|cell| cell.glyph));
        let (mosaics, letters) = glyphs.fold((0_usize, 0_usize), |(mosaics, letters), glyph| match glyph {
            Glyph::Mosaic { .. } => (mosaics.saturating_add(1), letters),
            Glyph::Char(_) => (mosaics, letters.saturating_add(1)),
            Glyph::Space => (mosaics, letters),
        });
        if mosaics.saturating_mul(MOSAIC_SHARE) > letters {
            continue;
        }
        let text: String = (0..COLUMNS)
            .map(|column| match grid.cell(row, column) {
                Some(cell) if !cell.conceal => match cell.glyph {
                    Glyph::Char(character) => to_unicode(character).base,
                    Glyph::Space | Glyph::Mosaic { .. } => ' ',
                },
                _ => ' ',
            })
            .collect();
        let words: Vec<&str> = text.split_whitespace().collect();
        if words.iter().flat_map(|word| word.chars()).filter(|character| character.is_alphanumeric()).count() >= TITLE_MINIMUM {
            return words.join(" ");
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two pages in TTI, the engine's page file format: page 100 with a
    /// coloured title, and page 101.
    const TTI: &[u8] = b"PN,10000\nSC,0000\nPS,8000\nOL,0,        HEADER\nOL,1,\x1b\x41RED HEADLINES\nOL,3,\x1b\x47News of the day\n\nPN,10100\nSC,0000\nPS,8000\nOL,1,Sport results\n";

    #[test]
    fn pages_arrive_with_titles_and_render_in_teletext_colours() {
        let mut engine = Engine::new();
        let mut arrived = engine.feed(TTI);
        arrived.extend(engine.finish());
        assert_eq!(arrived, [(0x100, "RED HEADLINES".to_owned()), (0x101, "Sport results".to_owned())]);
        let page = engine.render(0x100).unwrap();
        assert_eq!(page.indices.len(), PAGE_DOTS);
        assert!(page.palette.len() <= 32);
        assert!(page.indices.iter().all(|&index| usize::from(index) < page.palette.len()));
        // Some dot of row 1 is drawn in red: alpha-red is code 0x01 (Table 26).
        let row_one = &page.indices[usize::from(PAGE_WIDTH) * 20..usize::from(PAGE_WIDTH) * 40];
        assert!(row_one.iter().any(|&index| page.palette[usize::from(index)] == [0xFF, 0, 0]), "no red in row 1");
        assert!(engine.render(0x102).is_none(), "a page never received");
    }

    #[test]
    fn mosaics_are_not_taken_for_a_title() {
        // Row 1 is graphics (mosaic green, 0x12): its lower-case letters and
        // punctuation are mosaic blocks, not text. Capitals would still be
        // text there, by the blast-through rule of EN 300 706 Table 26.
        let mut engine = Engine::new();
        engine.feed(b"PN,10000\nSC,0000\nPS,8000\nOL,1,\x1bRppp ppp ,,|,,|\nOL,2,Front page\n");
        assert_eq!(engine.finish(), [(0x100, "Front page".to_owned())]);
    }

    #[test]
    fn a_picture_row_with_capitals_is_not_a_title() {
        // Row 1 is mostly mosaics with a few blast-through capitals.
        let mut engine = Engine::new();
        engine.feed(b"PN,10000\nSC,0000\nPS,8000\nOL,1,\x1bR,,|,,|E,,|C,,|,,|\nOL,2,Front page\n");
        assert_eq!(engine.finish(), [(0x100, "Front page".to_owned())]);
    }

    #[test]
    fn a_new_version_of_a_page_is_reported_as_updated() {
        let mut engine = Engine::new();
        engine.feed(TTI);
        engine.finish();
        assert_eq!(engine.take_updated(), [] as [u16; 0]);
        // The same page again, unchanged: no update.
        engine.feed(b"PN,10100\nSC,0000\nPS,8000\nOL,1,Sport results\n\n");
        engine.finish();
        assert_eq!(engine.take_updated(), [] as [u16; 0]);
        // A changed version: an update, and not a new arrival.
        let arrived = engine.feed(b"PN,10100\nSC,0000\nPS,8000\nOL,1,Sport results: 2-1\n\n");
        assert!(arrived.is_empty() && engine.finish().is_empty());
        assert_eq!(engine.take_updated(), [0x101]);
    }

    /// The audio packets of a 2 000-sample PCM sound at 8 kHz that plays on
    /// `page`, channel 1, written by the engine's encoder.
    fn sound_stream(page: u16) -> Vec<u8> {
        use neoteletext_audio::encode::{Command, Sound, packets};
        let pcm: Vec<i16> = (0..2000).map(|n| i16::try_from(n % 64).unwrap().saturating_mul(100)).collect();
        let sound = Sound::new(5, 1, neoteletext_audio::Codec::Pcm16, 8_000, 1000, &pcm).unwrap();
        let mut records = vec![sound.object_record().unwrap()];
        records.extend(sound.data_records());
        let play = Command { channel: 1, sequence: 1, operation: 1, handle: 5, generation: 1, gain: 70, fade_ms: 20, flags: 0, duck: 0, page: Some(page) };
        records.push(play.record());
        packets(0, 0, &records).0.iter().flat_map(|packet| packet.bytes).collect()
    }

    #[test]
    fn a_page_sound_is_sent_when_its_page_is_opened() {
        let mut engine = Engine::new();
        assert_eq!(engine.feed(&sound_stream(0x100)), [] as [(u16, String); 0], "audio packets are not pages");
        engine.finish();
        assert_eq!(engine.take_sounds(), [] as [Audio; 0], "nothing before the page is open");
        engine.open_page(Some(0x100));
        let sounds = engine.take_sounds();
        let sound = Sound { handle: 5, generation: 1 };
        assert_eq!(sounds.first(), Some(&Audio::Define { sound, rate_hz: 8_000, samples: 2000 }));
        let samples: usize = sounds.iter().map(|audio| if let Audio::Samples { pcm, .. } = audio { pcm.len() } else { 0 }).sum();
        assert_eq!(samples, 2000);
        assert!(matches!(sounds.last(), Some(Audio::Start { channel: 1, gain: 70, fade_ms: 20, .. })));
        // Leaving the page fades it out.
        engine.open_page(Some(0x101));
        assert_eq!(engine.take_sounds(), [Audio::Stop { channel: 1, fade_ms: 5 }]);
    }

    #[test]
    fn a_live_sound_streams_and_fades_two_seconds_after_its_last_block() {
        use neoteletext_audio::encode::{Command, Sound as Writer, packets};
        let live = Writer::live(6, 1, neoteletext_audio::Codec::Pcm16, 8_000, 100).unwrap();
        let play = Command { channel: 0, sequence: 1, operation: 1, handle: 6, generation: 1, gain: 50, fade_ms: 0, flags: 0, duck: 0, page: None };
        let mut records = vec![live.object_record().unwrap(), play.record()];
        for index in 0..4 {
            records.extend(live.live_records(index, &[100; 50]));
        }
        let bytes: Vec<u8> = packets(0, 0, &records).0.iter().flat_map(|packet| packet.bytes).collect();
        let mut engine = Engine::new();
        engine.tick_audio(1_000);
        engine.feed(&bytes);
        engine.finish();
        let sounds = engine.take_sounds();
        let sound = Sound { handle: 6, generation: 1 };
        assert_eq!(sounds.first(), Some(&Audio::Stream { sound, rate_hz: 8_000 }), "{sounds:?}");
        assert!(sounds.iter().any(|audio| matches!(audio, Audio::Start { channel: 0, .. })));
        assert!(sounds.iter().any(|audio| matches!(audio, Audio::StreamSamples { pcm, .. } if pcm.len() == 50)));
        engine.tick_audio(2_999);
        assert_eq!(engine.take_sounds(), [] as [Audio; 0]);
        engine.tick_audio(3_000);
        assert_eq!(engine.take_sounds(), [Audio::Stop { channel: 0, fade_ms: neoteletext_audio::LIVE_STALL_FADE_MS }, Audio::Forget(sound)]);
    }

    /// Decision 42: in NeoTeletext only links are underlined, so the page's
    /// own underline (a Level 2.5 Display Attribute) is not drawn.
    #[test]
    fn the_pages_own_underline_is_not_drawn() {
        use neoteletext_core::packet::DisplayAttributes;
        use neoteletext_formats::author::DisplayPage;
        let mut page = DisplayPage::new(PageAddress::from_mpp(0x100).unwrap(), SubCode::new(0));
        page.text(3, 4, b"UNDERLINE").unwrap();
        page.local().position(3, 0).unwrap().attributes(4, DisplayAttributes { underline: true, ..DisplayAttributes::default() });
        let mut bytes = Vec::new();
        neoteletext_formats::write::write_tti(&[&page.build().unwrap()], &mut bytes).unwrap();
        let mut engine = Engine::new();
        engine.feed(&bytes);
        engine.finish();
        let rendered = engine.render(0x100).unwrap();
        let (grid, _) = engine.grid(0x100).unwrap();
        assert!(grid.cell(3, 4).is_some_and(|cell| cell.attributes.underline()), "the page does underline the word");
        // The last line of row 3 under the word, which an underline fills.
        let width = usize::from(PAGE_WIDTH);
        let cell = usize::from(interface_core::canvas::CELL_WIDTH);
        let height = usize::from(interface_core::canvas::CELL_HEIGHT);
        let line = 3 * height + height - 1;
        let under = &rendered.indices[line * width + 4 * cell..line * width + 13 * cell];
        // Columns 0 to 3 of the row are not underlined: their last line is
        // the row's background.
        let background = rendered.indices[line * width];
        assert!(under.iter().all(|&index| index == background), "no line under the text");
    }

    /// `hyperlinks.md` section 6: the page numbers in a page's text and its
    /// `LT` and `LK` lines are found, for the trusted core to decide.
    #[cfg(feature = "links")]
    #[test]
    fn a_pages_links_are_found_in_its_text_and_its_link_lines() {
        use protocol::links::{Candidate, Found, NumberLink};
        let page = b"PN,10000\nSC,0000\nPS,8000\nOL,1,News index\nOL,3,Sport 101\nOL,5,Weather here\n\
LT,5,0,7,web:https://www.example.invalid/\nLK,101,https://sport.invalid/\n";
        let mut engine = Engine::new();
        engine.feed(page);
        engine.finish();
        let links = engine.render(0x100).unwrap().links;
        let at = |candidate: &Candidate| (candidate.row, candidate.column, candidate.length, candidate.target.clone());
        assert_eq!(
            links.candidates.iter().map(at).collect::<Vec<_>>(),
            [(5, 0, 7, Found::Declared("web:https://www.example.invalid/".into())), (3, 6, 3, Found::Number(0x101))]
        );
        assert_eq!(links.numbers, [NumberLink { number: 0x101, target: "https://sport.invalid/".into() }]);
        // A run over blank cells is trimmed to the text it covers.
        let mut engine = Engine::new();
        engine.feed(b"PN,10000\nSC,0000\nPS,8000\nOL,5,   Weather\nLT,5,0,20,302\nLT,7,0,10,303\n");
        engine.finish();
        let links = engine.render(0x100).unwrap().links;
        assert_eq!(links.candidates.iter().map(at).collect::<Vec<_>>(), [(5, 3, 7, Found::Declared("302".into()))], "row 7 is blank");
    }

    /// `interaction.md` section 10: a pushed page shows when its message
    /// arrives, not when the next page of its magazine begins; and a channel
    /// may push packets after fetching page files.
    #[test]
    fn a_pushed_page_shows_as_it_arrives() {
        let text = |engine: &Engine| -> String {
            let (grid, _) = engine.grid(0x100).unwrap();
            (0..11)
                .map(|column| match grid.cell(1, column).unwrap().glyph {
                    Glyph::Char(character) => to_unicode(character).base,
                    _ => ' ',
                })
                .collect()
        };
        let mut engine = Engine::new();
        engine.feed(TTI);
        engine.finish();
        assert_eq!(engine.feed_whole(false, true, b"PN,10000\nSC,0000\nPS,8000\nOL,1,Version one\n"), [] as [(u16, String); 0], "an update, not an arrival");
        assert_eq!(text(&engine), "Version one");
        assert_eq!(engine.take_updated(), [0x100]);
        engine.feed_whole(false, true, b"PN,10000\nSC,0000\nPS,8000\nOL,1,Version two\n");
        assert_eq!(text(&engine), "Version two");
        // The same page pushed as packets: the TTI page encoded by the engine.
        let mut packets = Vec::new();
        let mut reader = AutoReader::new(AutoConfig { format: Some(Format::Tti), encode_pages: true, ..AutoConfig::default() });
        let mut take = |item: Item<'_>| {
            if let Item::Packet { packet, .. } = item {
                packets.extend_from_slice(&packet.bytes);
            }
        };
        reader.feed(b"PN,10000\nSC,0000\nPS,8000\nOL,1,Packet push\n", &mut take);
        reader.finish(&mut take);
        engine.feed_whole(true, true, &packets);
        assert_eq!(text(&engine), "Packet push");
        // A fetched copy of a pushed page, older, does not replace it,
        // whether it comes whole or in pieces.
        engine.feed_whole(false, false, b"PN,10000\nSC,0000\nPS,8000\nOL,1,Fetched old\n");
        assert_eq!(text(&engine), "Packet push");
        engine.feed(b"PN,10000\nSC,0000\nPS,8000\nOL,1,Fetched old\n");
        engine.finish();
        assert_eq!(text(&engine), "Packet push");
        // A fetched file's last page arrives with the file, though no
        // other page of its magazine follows.
        let arrived = engine.feed_whole(false, false, b"PN,30000\nSC,0000\nPS,8000\nOL,1,Third magazine\n");
        assert_eq!(arrived, [(0x300, "Third magazine".to_owned())]);
    }

    /// REQ-INT-110, REQ-INT-111 and REQ-INT-114: a page's `CU` and `IM`
    /// lines, and a pushed cursor or control record, reach the trusted
    /// core as bodies; nothing else does.
    #[cfg(feature = "interaction")]
    #[test]
    fn the_channels_input_records_are_passed_on() {
        use neoteletext_input::control::{Control, packet_of, read_body};
        use neoteletext_input::modes::MOUSE_BUTTONS;
        let mut engine = Engine::new();
        engine.feed_whole(false, true, b"PN,50000\r\nOL,1,Links\r\nCU,255,0,0,0,7\r\nIM,8\r\n");
        let input = engine.take_input();
        assert_eq!(input.cursors, [[0x05, 0x00, 0, 7, 255, 0, 0, 0]]);
        assert_eq!(input.controls.iter().map(|body| read_body(body).unwrap()).collect::<Vec<_>>(), [Control::PageModes { page: 0x500, modes: MOUSE_BUTTONS }]);
        let welcome = packet_of(&Control::Welcome { version: 2, caps: MOUSE_BUTTONS }, 0, 1).unwrap();
        engine.feed_whole(true, true, &welcome.bytes);
        let input = engine.take_input();
        assert_eq!(input.controls.iter().map(|body| read_body(body).unwrap()).collect::<Vec<_>>(), [Control::Welcome { version: 2, caps: MOUSE_BUTTONS }]);
        assert_eq!(input.cursors, [] as [[u8; neoteletext_input::cursor::BODY]; 0]);
        assert_eq!(engine.take_input(), InputRecords::default(), "once each");
        engine.feed_whole(false, true, b"PN,50100\r\nOL,1,None\r\n");
        assert_eq!(engine.take_input(), InputRecords::default(), "a page without the lines");
    }

    #[test]
    fn a_message_reports_only_the_pages_it_touched() {
        let mut engine = Engine::new();
        engine.feed_whole(false, false, TTI);
        assert_eq!(engine.take_changed().pages, BTreeSet::from([0x100, 0x101]));
        engine.feed_whole(false, false, b"PN,10100\nSC,0000\nPS,8000\nOL,1,Sport results: 2-1\n\n");
        let changed = engine.take_changed();
        assert_eq!(changed.pages, BTreeSet::from([0x101]), "only the page in the message");
        assert!(!changed.everything);
        assert_eq!(engine.take_changed(), Changed::default(), "once each");
    }

    #[test]
    fn arrivals_compare_only_the_pages_that_completed() {
        // A store of many pages: one page more must not compare them all,
        // as scanning the store on every message did (1.5 ms a message at
        // 4,096 pages, release build, 21 October 2026).
        const PAGES_STORED: u16 = 200;
        let mut engine = Engine::new();
        let numbers: Vec<u16> = (0..PAGES_STORED).map(|offset| 0x100 + offset / 10 * 0x10 + offset % 10).collect();
        engine.feed_whole(false, false, &sample::tti(&numbers, &[]));
        let before = engine.digests;
        engine.feed_whole(false, false, b"PN,10100\nSC,0000\nPS,8000\nOL,1,One page more\n\n");
        assert_eq!(engine.digests - before, 1, "only the page that completed");
        assert_eq!(engine.take_updated(), [0x101]);
    }

    #[test]
    fn only_a_shown_page_that_flashes_wakes_the_decoder_and_only_when_its_phase_changes() {
        let mut engine = Engine::new();
        engine.feed_whole(false, false, b"PN,10000\nSC,0000\nPS,8000\nOL,1,Steady\n\nPN,10100\nSC,0000\nPS,8000\nOL,1,\x1b\x48Flashing\n\n");
        let mut flashing = Flashing::default();
        // Unit 0 of a cycle: slow flash on (decision E.48).
        let start = 3 * FLASH_CYCLE_US;
        flashing.drawn(0x100, &engine.render_at(0x100, start).unwrap());
        assert_eq!(flashing.next_us(start), None, "a steady page never wakes it");
        flashing.drawn(0x101, &engine.render_at(0x101, start).unwrap());
        let unit = FLASH_CYCLE_US.div_ceil(u64::from(FLASH_UNITS));
        // Slow flash, the only rate the page uses, stays on until unit 9:
        // the fast rates change phase before then, but change nothing on
        // this page, so the decoder sleeps until unit 9. Before this it
        // woke at every unit and drew the page at seven of the twelve.
        assert_eq!(flashing.next_us(start), Some(start + (9 * FLASH_CYCLE_US).div_ceil(u64::from(FLASH_UNITS))), "unit 9, where the slow flash turns off");
        assert_eq!(flashing.due(start + unit), [] as [u16; 0]);
        assert_eq!(flashing.due(start + 2 * unit + 1), [] as [u16; 0], "the fast rates' change at unit 2 is not this page's");
        let off = start + 9 * FLASH_CYCLE_US / u64::from(FLASH_UNITS) + 1;
        assert_eq!(flashing.due(off), [0x101]);
        flashing.retain(&[0x100]);
        assert_eq!(flashing.next_us(off), None, "no longer shown");
    }

    #[test]
    fn a_store_that_fills_is_reported_once() {
        const ROOM: usize = 4;
        let mut engine = Engine::with_store(ROOM);
        engine.feed_whole(false, false, &sample::tti(&[0x100, 0x101, 0x102], &[]));
        engine.take_changed();
        assert!(!engine.take_store_full(), "room for one more");
        engine.feed_whole(false, false, &sample::tti(&[0x103, 0x104], &[]));
        engine.take_changed();
        assert!(engine.take_store_full(), "as many pages as the store has room for");
        assert!(!engine.take_store_full(), "once");
    }

    #[test]
    fn arrivals_are_reported_once() {
        let mut engine = Engine::new();
        engine.feed(TTI);
        engine.finish();
        assert_eq!(engine.feed(b""), [] as [(u16, String); 0]);
        assert_eq!(engine.finish(), [] as [(u16, String); 0]);
    }
}

/// Sample page files for examples and tests: TTI, the engine's page file
/// format, with a coloured title, a few lines of text and a band of
/// mosaics on each page.
pub mod sample {
    use std::fmt::Write;

    /// Alpha colour codes 1 to 7 (EN 300 706 Table 26), written as TTI's
    /// escape followed by the code plus 0x40.
    const COLOURS: [char; 6] = ['A', 'B', 'C', 'D', 'E', 'F'];
    /// Mosaic colour codes 0x11 to 0x17, and a run of solid mosaic blocks.
    const MOSAICS: [char; 6] = ['Q', 'R', 'S', 'T', 'U', 'V'];
    const SOLID_BLOCKS: &str = "\x7f\x7f\x7f\x7f\x7f\x7f\x7f\x7f\x7f\x7f";

    /// One TTI page per number in `pages`, titled `titles[i % len]`.
    #[must_use]
    pub fn tti(pages: &[u16], titles: &[&str]) -> Vec<u8> {
        let mut out = String::new();
        // With no titles, every page is untitled.
        let titles = titles.iter().copied().cycle().chain(core::iter::repeat(""));
        for (((&page, colour), mosaic), title) in pages.iter().zip(COLOURS.iter().cycle()).zip(MOSAICS.iter().cycle().skip(1)).zip(titles) {
            let _ = write!(
                out,
                "PN,{page:03X}00\nSC,0000\nPS,8000\nOL,1,\x1b{colour}\x1b\x4d{title}\nOL,4,\x1bGPage {page:03X} of the sample channel\nOL,6,\x1bFThe teletext engine decoded and drew this.\nOL,10,\x1b{mosaic}{SOLID_BLOCKS}\nOL,11,\x1b{mosaic}{SOLID_BLOCKS}\n\n"
            );
        }
        out.into_bytes()
    }
}

#[cfg(feature = "interaction")]
impl InputRecords {
    /// A cursor or control record's packet: its body kept when it reads.
    fn take_packet(&mut self, packet: &Packet) {
        use neoteletext_input::control;
        if let Some(Ok(cursor)) = neoteletext_input::cursor::from_packet(packet) {
            self.cursors.push(cursor.body());
            return;
        }
        let mut buffer = [0_u8; control::FRAGMENT_DATA];
        if let Some(Ok(message)) = control::from_packet(packet, &mut buffer) {
            self.control(&message);
        }
    }

    fn page_modes(&mut self, page: u16, modes: u32) {
        self.control(&neoteletext_input::control::Control::PageModes { page, modes });
    }

    fn control(&mut self, message: &neoteletext_input::control::Control<'_>) {
        let mut body = [0_u8; neoteletext_input::control::MAX_BODY];
        if let Ok(length) = neoteletext_input::control::write_body(message, &mut body) {
            self.controls.push(body.get(..length).unwrap_or(&[]).to_vec());
        }
    }
}
