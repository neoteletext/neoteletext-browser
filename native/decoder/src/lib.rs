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

use std::time::Instant as Clock;

use interface_core::canvas::{PAGE_HEIGHT, PAGE_WIDTH};
use interface_core::channel::{PAGES, page_index};
use neoteletext_audio::port::{SoundFormat, SoundId, Start, VoiceSink};
use neoteletext_audio::receiver::Receiver;
use neoteletext_core::acq::{Decoder, EnhancementPacket, EnhancementStore, EnhancementSubPage, StoredPage};
use neoteletext_core::charset::to_unicode;
use neoteletext_core::compose::compose_level;
use neoteletext_core::config::Config;
use neoteletext_core::consts::{COLUMNS, ROWS};
use neoteletext_core::consts::{ENHANCEMENT_PACKETS_LEVEL_35, MAGAZINES, MOT_PACKETS, PAGE_COMPLETION_TIMEOUT_US};
use neoteletext_core::fsm::PageState;
use neoteletext_core::glyphs::GlyphSet;
use neoteletext_core::level1::{DisplayOptions, Glyph, Grid, build_grid};
use neoteletext_core::packet::{Packet, PacketMeta};
use neoteletext_core::render::{FlashPhase, IndexBuffer, IndexFormat, RenderOptions, render_screen};
use neoteletext_core::types::{Instant, PageAddress, SubCode};
use neoteletext_formats::Item;
use neoteletext_formats::detect::{AutoConfig, AutoReader};
use neoteletext_formats::item::Reader;
use neoteletext_image::palette::{Rgba, colour_map_palette};
use protocol::audio::{Audio, MAX_MESSAGE_SAMPLES, Sound};

/// Stored pages and sub-pages, as the engine's own sessions keep
/// (`neoteletext-app`'s `DEFAULT_PAGES`).
const STORE_PAGES: usize = 4096;
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
}

/// The teletext engine for one channel: it reads the channel's bytes,
/// keeps its pages, reports which have arrived or changed, and renders them.
pub struct Engine {
    decoder: Decoder<'static>,
    reader: AutoReader,
    clock: u64,
    /// The display pages already reported as arrived, each with a digest
    /// of its content, to tell when a new version changes it.
    reported: Vec<Option<u64>>,
    started: Clock,
    /// Pages already reported whose content has since changed.
    updated: Vec<u16>,
    /// The channel's sounds: the audio packets go here, not to the decoder.
    audio: Receiver<'static, AUDIO_OBJECTS, AUDIO_RECORDS>,
    /// Voice operations waiting to be sent.
    sounds: Vec<Audio>,
    /// The page the user opened, if any.
    opened: Option<u16>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// An engine with an empty page store.
    #[must_use]
    pub fn new() -> Self {
        // One store for the life of the process, as the engine's sessions do.
        let slots: &'static mut [StoredPage] = Box::leak(vec![StoredPage::EMPTY; STORE_PAGES].into_boxed_slice());
        let sub_pages = Box::leak(vec![EnhancementSubPage::EMPTY; ENHANCEMENT_SUB_PAGES].into_boxed_slice());
        let packets = Box::leak(vec![EnhancementPacket::EMPTY; ENHANCEMENT_PACKETS].into_boxed_slice());
        let decoder = Decoder::with_enhancement_store(slots, EnhancementStore::new(sub_pages, packets), Config::default());
        // Detection decides the format; page files are encoded into
        // packets, so they take the same path as broadcasts.
        let reader = AutoReader::new(AutoConfig { encode_pages: true, ..AutoConfig::default() });
        let arena: &'static mut [u8] = Box::leak(vec![0_u8; AUDIO_ARENA].into_boxed_slice());
        Self {
            decoder,
            reader,
            clock: 0,
            reported: vec![None; PAGES],
            started: Clock::now(),
            updated: Vec::new(),
            audio: Receiver::new(arena),
            sounds: Vec::new(),
            opened: None,
        }
    }

    /// The voice operations produced since the last call, once each.
    pub fn take_sounds(&mut self) -> Vec<Audio> {
        core::mem::take(&mut self.sounds)
    }

    /// The user opened `page`, or none: its sounds play while it is open
    /// (the engine's REQ-AUD-012).
    pub fn open_page(&mut self, page: Option<u16>) {
        self.opened = page;
        self.audio.open_page(page, &mut Outbox(&mut self.sounds));
    }

    /// Whether the channel is the one heard. Becoming heard, its sounds are
    /// sent again, because the interface forgot them, and the open page's
    /// sounds start again.
    pub fn set_audible(&mut self, heard: bool) {
        if heard {
            self.audio.sink_lost();
            let mut outbox = Outbox(&mut self.sounds);
            self.audio.open_page(None, &mut outbox);
            self.audio.open_page(self.opened, &mut outbox);
        }
    }

    fn push_all(&mut self, packets: &[(Packet, PacketMeta)]) {
        for (packet, meta) in packets {
            // The audio profile's packets go to its receiver; the decoder
            // ignores packet 31 anyway (the engine's REQ-PKT-008).
            if self.audio.push_packet(packet, &mut Outbox(&mut self.sounds)) {
                continue;
            }
            self.clock = match meta.timestamp_us {
                Some(stamp) if stamp >= self.clock => stamp,
                _ => self.clock.saturating_add(LINE_PERIOD_US),
            };
            self.decoder.push(packet, meta, Instant(self.clock), &mut |_| {});
        }
    }

    /// Feeds more of the channel's bytes; returns the pages that arrived.
    /// Pages that changed are kept for [`Engine::take_updated`].
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<(u16, String)> {
        let mut packets = Vec::new();
        self.reader.feed(bytes, &mut |item| {
            if let Item::Packet { packet, meta } = item {
                packets.push((packet, meta));
            }
        });
        self.push_all(&packets);
        self.arrivals()
    }

    /// The pages whose content changed since they were reported, once each.
    pub fn take_updated(&mut self) -> Vec<u16> {
        core::mem::take(&mut self.updated)
    }

    /// Ends the channel's data: completes every page still being received.
    pub fn finish(&mut self) -> Vec<(u16, String)> {
        let mut packets = Vec::new();
        self.reader.finish(&mut |item| {
            if let Item::Packet { packet, meta } = item {
                packets.push((packet, meta));
            }
        });
        self.push_all(&packets);
        self.clock = self.clock.saturating_add(PAGE_COMPLETION_TIMEOUT_US.saturating_mul(2));
        self.decoder.tick(Instant(self.clock), &mut |_| {});
        self.arrivals()
    }

    /// Display pages received completely and not yet reported, each with
    /// a title from its text.
    fn arrivals(&mut self) -> Vec<(u16, String)> {
        let mut fresh = Vec::new();
        for page in self.decoder.store().pages() {
            if page.state != PageState::Complete {
                continue;
            }
            let number = page.key.address.mpp();
            let Some(index) = page_index(number) else { continue };
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
        fresh.into_iter().map(|page| (page, self.grid(page).map(|grid| title(&grid)).unwrap_or_default())).collect()
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
                Exported { page: number, rows, title: self.grid(number).map(|grid| title(&grid)).unwrap_or_default() }
            })
            .collect();
        pages.sort_by_key(|page| page.page);
        pages.dedup_by_key(|page| page.page);
        pages
    }

    /// The page's grid, as the engine composes it for display.
    fn grid(&self, page: u16) -> Option<Grid> {
        let address = PageAddress::from_mpp(page)?;
        let stored = self.decoder.store().get(address, SubCode::ANY)?;
        let config = self.decoder.config();
        let magazine = self.decoder.magazine_data(address.magazine);
        let options = DisplayOptions::default();
        let mut grid = Grid::EMPTY;
        build_grid(&mut grid, Some(stored), &magazine, config, &options);
        let lookup = self.decoder.enhancement_lookup();
        let _ = compose_level(&mut grid, stored, &magazine, &lookup, config, &options);
        Some(grid)
    }

    /// Renders the latest sub-page of `page` at the engine's flash phase
    /// for now, in the resolved colour mode, whose palette is the page's
    /// Colour Map of at most 32 colours.
    #[must_use]
    pub fn render(&self, page: u16) -> Option<Rendered> {
        page_index(page)?;
        let grid = self.grid(page)?;
        let config = self.decoder.config();
        let elapsed = u64::try_from(self.started.elapsed().as_micros()).unwrap_or(u64::MAX);
        let render = RenderOptions::resolved(config, false, FlashPhase::at(Instant(elapsed)));
        let mut indices = vec![0_u8; PAGE_DOTS];
        let mut buffer = IndexBuffer { format: IndexFormat::Bits8, data: &mut indices, stride: usize::from(PAGE_WIDTH) };
        render_screen(&grid, ALL_ROWS, &GLYPHS, render, &mut buffer).ok()?;
        let palette = colour_map_palette(&grid.colour_map, PICTURE).iter().map(|colour| [colour.r, colour.g, colour.b]).collect();
        Some(Rendered { palette, indices })
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
    fn becoming_heard_sends_the_open_pages_sounds_again() {
        let mut engine = Engine::new();
        // Detection holds the bytes until the stream is long enough or ends.
        engine.feed(&sound_stream(0x100));
        engine.finish();
        engine.open_page(Some(0x100));
        assert!(matches!(engine.take_sounds().last(), Some(Audio::Start { .. })));
        engine.set_audible(false);
        assert_eq!(engine.take_sounds(), [] as [Audio; 0]);
        engine.set_audible(true);
        let sounds = engine.take_sounds();
        assert!(sounds.iter().any(|audio| matches!(audio, Audio::Define { .. })), "defined again: {sounds:?}");
        assert!(matches!(sounds.last(), Some(Audio::Start { channel: 1, .. })));
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
