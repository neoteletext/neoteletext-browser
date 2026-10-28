//! The engine for one channel: it reads the channel's bytes, keeps its
//! pages, reports which have arrived or changed, and renders them.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant as Clock;

use interface_core::canvas::{PAGE_HEIGHT, PAGE_WIDTH};
use interface_core::channel::PAGES;
use neoteletext_audio::receiver::Receiver;
use neoteletext_core::acq::{Decoder, EnhancementPacket, EnhancementStore, EnhancementSubPage, StoredPage};
use neoteletext_core::config::Config;
use neoteletext_core::consts::COLUMNS;
use neoteletext_core::consts::{ENHANCEMENT_PACKETS_LEVEL_35, MAGAZINES, MOT_PACKETS};
use neoteletext_core::glyphs::GlyphSet;
use neoteletext_core::render::{ChangeTracker, FlashPhase};
use neoteletext_formats::detect::{AutoConfig, AutoReader, Format};
use neoteletext_formats::t42::T42Variant;
use neoteletext_image::palette::Rgba;
use protocol::audio::Audio;
use protocol::decoder::{FIRST_STORE_PAGES, MAX_STORE_PAGES, View};
use std::cell::RefCell;

#[cfg(feature = "interaction")]
use crate::InputRecords;
use crate::changes::Changes;
#[cfg(feature = "links")]
use crate::links;
use crate::voice_sink::Outbox;
use crate::{Changed, FlashRates};

mod feed;
mod render;
#[cfg(test)]
mod tests;

use feed::ReceiverRow;

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
    /// The page's sub-pages from the one drawn: their count, its place and
    /// its neighbours (the engine's REQ-NAV-005).
    pub subpages: protocol::subpages::SubPageStep,
}

/// The links of a page as the decoder finds them, untrusted.
#[cfg(feature = "links")]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    /// Page numbers in its text and runs its `LT` lines declare.
    pub candidates: Vec<protocol::links::Candidate>,
    /// Its `LK` lines.
    pub numbers: Vec<protocol::links::NumberLink>,
    /// Its controls, as the engine finds them in its packets X/26
    /// (INPUT.md section 7.3, the engine's decision E.127).
    pub controls: Vec<protocol::links::Area>,
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
    /// Each page's count of sub-pages as last told to the trusted core, for
    /// the page list: only those with two or more.
    counts_said: BTreeMap<u16, u16>,
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
    /// The area of a page under the pointer and whether it is pressed:
    /// drawn in its look (INPUT.md section 9.7).
    #[cfg(feature = "links")]
    looks: BTreeMap<u16, (protocol::links::Area, bool)>,
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
            counts_said: BTreeMap::new(),
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
            #[cfg(feature = "links")]
            looks: BTreeMap::new(),
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
        self.audio.open_page(page, &mut Outbox::new(&mut self.sounds));
    }

    /// The time, in milliseconds from any start: a live sound with no new
    /// block for two seconds fades out (the engine's REQ-AUD-014). Called
    /// before each piece of data and at each tick.
    pub fn tick_audio(&mut self, now_ms: u64) {
        self.audio.tick(now_ms, &mut Outbox::new(&mut self.sounds));
    }

    /// The channel's records for its input found since the last call, once
    /// each.
    #[cfg(feature = "interaction")]
    pub fn take_input(&mut self) -> InputRecords {
        core::mem::take(&mut self.input)
    }

    /// The pages whose packets arrived, or whose reception started, ended
    /// or was erased, since the last call; and whether data every page may
    /// use changed (a magazine's or an enhancement page's), when every
    /// shown page is to be drawn again.
    pub fn take_changed(&mut self) -> Changed {
        let (started, changed) = self.changes.take_changed();
        self.started_pages.extend(started);
        changed
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
}
