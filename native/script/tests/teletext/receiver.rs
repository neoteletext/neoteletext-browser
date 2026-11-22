//! A receiver for the teletext tests: the engine itself, given the page a
//! script's batches describe as the packets a decoder would build from
//! them (script-api.md section 11.2), so that what a test keeps is what a
//! viewer would be shown at Level 2.5 and not only what the host wrote.

use std::fmt::Write as _;

use neoteletext_core::acq::{Decoder, EnhancementPacket, EnhancementStore, EnhancementSubPage, StoredPage};
use neoteletext_core::charset::language::language_value;
use neoteletext_core::charset::to_unicode;
use neoteletext_core::config::{Config, PresentationLevel};
use neoteletext_core::consts::{
    COLOUR_MAP_DEFAULTS, COLUMNS, DATA_BYTES, HEADER_DISPLAY_BYTES, MAGAZINES, NO_SECOND_G0, PACKET_ENHANCEMENT, PAGE_TIME_FILLING, SPACE,
};
use neoteletext_core::ecc::with_odd_parity;
use neoteletext_core::enhance;
use neoteletext_core::level1::{DisplayOptions, Glyph, Grid, build_grid};
use neoteletext_core::packet::{Packet, PacketMeta, PageFormat, Triplet, encode};
use neoteletext_core::types::{ControlBits, DefaultColourFields, Designations, Instant, Magazine, PageAddress, PresentationFields, Rgb12, SubCode};
use neoteletext_formats::write_text::packets as enhancement_packets;
use protocol::script::Batch;
use protocol::script::batch::{FIRST_DEFINABLE_ENTRY, FIRST_SCRIPT_ROW, MAX_SCRIPT_ROWS, TeletextSettings};

/// The page the tests show.
const PAGE: u16 = 0x100;
/// The time between two packets fed, and the time the pages are closed at.
const PACKET_INTERVAL: u64 = 64;
const CLOSING_TIME: u64 = 1_000_000_000;

/// The page as a script's batches have left it.
pub struct Receiver {
    rows: [[u8; COLUMNS]; MAX_SCRIPT_ROWS],
    triplets: Vec<Triplet>,
    settings: TeletextSettings,
    language: &'static str,
}

impl Receiver {
    /// A blank page in `language`.
    pub fn new(language: &'static str) -> Self {
        Self { rows: [[SPACE; COLUMNS]; MAX_SCRIPT_ROWS], triplets: Vec::new(), settings: TeletextSettings::default(), language }
    }

    /// Takes a step's batch as the decoder does: the rows it carries
    /// replace those rows, and the enhancement data and settings replace
    /// the page's whole.
    pub fn take(&mut self, batch: &Batch) {
        for row in batch.teletext_rows.iter().flatten() {
            self.rows[usize::from(row.row - FIRST_SCRIPT_ROW)] = row.codes;
        }
        if let Some(triplets) = &batch.teletext_enhancement {
            self.triplets = triplets.iter().map(|triplet| Triplet { address: triplet.address, mode: triplet.mode, data: triplet.data }).collect();
        }
        if let Some(settings) = &batch.teletext_settings {
            self.settings = settings.clone();
        }
    }

    /// The page's packets: a header, its rows, its packets X/26 and its
    /// packet X/28/0 format 1.
    pub fn packets(&self) -> Vec<Packet> {
        let address = PageAddress::from_mpp(PAGE).unwrap();
        let header = |mpp: u16| {
            let address = PageAddress::from_mpp(mpp).unwrap();
            encode::header(address.magazine, address.page, SubCode::new(0), ControlBits(0), &[with_odd_parity(SPACE); HEADER_DISPLAY_BYTES]).unwrap()
        };
        let mut packets = vec![header(PAGE)];
        let mut words = [Rgb12(0); 16];
        for (word, default) in words.iter_mut().zip(&COLOUR_MAP_DEFAULTS[usize::from(FIRST_DEFINABLE_ENTRY)..]) {
            *word = Rgb12(*default);
        }
        for (entry, colour) in &self.settings.colours {
            words[usize::from(entry - FIRST_DEFINABLE_ENTRY)] = Rgb12(*colour);
        }
        let fields = PresentationFields {
            designations: Some(Designations { default: language_value(self.language, true).unwrap(), second: NO_SECOND_G0 }),
            side_panels: None,
            colour_words: Some(words),
            colours: Some(DefaultColourFields {
                screen: self.settings.screen_colour,
                row: self.settings.row_colour,
                black_background_substitution: self.settings.substitution,
                remapping: 0,
            }),
        };
        packets.push(encode::page_enhancement(address.magazine, 0, PageFormat { function: 0, coding: 0 }, &fields));
        for (designation, triplets) in (0_u8..).zip(enhancement_packets(&self.triplets)) {
            packets.push(encode::triplet_packet(address.magazine, PACKET_ENHANCEMENT, designation, &triplets));
        }
        for (row, codes) in (FIRST_SCRIPT_ROW..).zip(&self.rows) {
            let mut bytes = [with_odd_parity(SPACE); DATA_BYTES];
            for (slot, code) in bytes.iter_mut().zip(codes) {
                *slot = with_odd_parity(*code);
            }
            packets.push(encode::row(address.magazine, row, &bytes));
        }
        // A time-filling header in each magazine ends its page.
        for magazine in 1..=u16::try_from(MAGAZINES).unwrap() {
            packets.push(header(magazine << 8 | u16::from(PAGE_TIME_FILLING)));
        }
        packets
    }

    /// The page as a Level 2.5 receiver resolves it.
    pub fn grid(&self) -> Box<Grid> {
        self.grid_at(PresentationLevel::Level25)
    }

    /// The page as a receiver of `level` resolves it.
    pub fn grid_at(&self, level: PresentationLevel) -> Box<Grid> {
        let config = Config { level, ..Config::default() };
        let mut pages = vec![StoredPage::EMPTY; 16];
        let mut sub_pages = vec![EnhancementSubPage::EMPTY; 16];
        let mut held = vec![EnhancementPacket::EMPTY; 64];
        let mut decoder = Decoder::with_enhancement_store(&mut pages, EnhancementStore::new(&mut sub_pages, &mut held), config);
        for (at, packet) in (0_u64..).zip(&self.packets()) {
            decoder.push(packet, &PacketMeta::default(), Instant(at * PACKET_INTERVAL), &mut |_| {});
        }
        decoder.tick(Instant(CLOSING_TIME), &mut |_| {});
        let address = PageAddress::from_mpp(PAGE).unwrap();
        let page = decoder.store().get(address, SubCode::ANY).expect("the page is stored");
        let magazine = decoder.magazine_data(Magazine::new(1).unwrap());
        let mut grid = Box::new(Grid::EMPTY);
        let options = DisplayOptions::default();
        build_grid(&mut grid, Some(page), &magazine, &config, &options);
        let _ = enhance::apply(&mut grid, page, &magazine, &decoder.enhancement_lookup(), &config, &options);
        grid
    }

    /// The page as text a test keeps: for each row 1 to 23 its
    /// characters (`#` for a mosaic), then each cell's foreground and
    /// background as a Colour Map entry in base 32, and `F` under a cell
    /// that flashes.
    pub fn picture(&self) -> String {
        Self::picture_of(&self.grid())
    }

    /// A resolved page as the text a test keeps.
    pub fn picture_of(grid: &Grid) -> String {
        let digit = |entry: u8| char::from_digit(u32::from(entry), 32).unwrap();
        let mut out = String::new();
        for (row, cells) in grid.cells.iter().enumerate().skip(usize::from(FIRST_SCRIPT_ROW)).take(MAX_SCRIPT_ROWS) {
            let text: String = cells
                .iter()
                .map(|cell| match cell.glyph {
                    Glyph::Char(character) => to_unicode(character).base,
                    Glyph::Mosaic { .. } => '#',
                    Glyph::Space => ' ',
                })
                .collect();
            let foreground: String = cells.iter().map(|cell| digit(cell.foreground)).collect();
            let background: String = cells.iter().map(|cell| digit(cell.background)).collect();
            let flash: String = cells.iter().map(|cell| if cell.flash { 'F' } else { '.' }).collect();
            let _ = write!(out, "{row:02} text |{text}|\n{row:02} ink  |{foreground}|\n{row:02} paper|{background}|\n{row:02} flash|{flash}|\n");
        }
        out
    }
}
