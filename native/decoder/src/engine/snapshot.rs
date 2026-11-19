//! A snapshot drawn (neoteletext-common's `docs/tab-thumbnails.md`
//! section 10, REQ-6885): the one sub-page asked for, as a still, and its
//! source in a static channel's sub-page file form with every concealed
//! character no cell shows blanked, so that what the store keeps holds no
//! concealed text (DEC-6875 item 7).
//!
//! The concealed places are found from this engine's own composed page:
//! the grid as the page is drawn, whose cells carry their concealment
//! (EN 300 706 clause 12.2's Conceal Display, clause 12.3's display
//! attributes). A Level 1 code under a concealed cell becomes a space; an
//! enhancement triplet (X/26) whose character lands in a concealed cell is
//! found by writing a space in its place and seeing which cells change,
//! and is kept so when one of them is concealed. Each change is kept only
//! when the code it hides is shown by no cell. The result is checked whole
//! before it is given: drawn with reveal off it must give the original's
//! image, and with every concealed cell revealed it must show no glyph the
//! original shows in no cell; a page that fails is refused, never stored.

use neoteletext_core::acq::StoredPage;
use neoteletext_core::config::ParityPolicy;
use neoteletext_core::consts::{
    CODE_MASK_7BIT, COLUMNS, HEADER_DISPLAY_COLUMN, MODE_DIACRITIC_MASK, MODE_DRCS_CHARACTER, MODE_G0_LEVEL_25, MODE_G0_NO_DIACRITIC, MODE_G1, MODE_G2,
    MODE_G3_LEVEL_15, MODE_G3_LEVEL_25, ROW_ADDRESS_BASE, SPACE, STORED_ROWS,
};
use neoteletext_core::fsm::PageState;
use neoteletext_core::level1::{Glyph, Grid};
use neoteletext_core::limits::SUBCODE_ANY;
use neoteletext_core::packet::Packet;
use neoteletext_core::render::{FlashPhase, IndexBuffer, IndexFormat, RenderOptions, render_screen};
use neoteletext_core::types::{PageAddress, SubCode};
use neoteletext_formats::page::{EnhancementPacket, FastextLink, PageFile};
use neoteletext_image::palette::colour_map_palette;
use protocol::decoder::{SnapshotImage, SnapshotRefusal, View};
use protocol::snapshot::MAX_SNAPSHOT_BYTES;

use super::render::GLYPHS;
use super::{ALL_ROWS, Engine, PAGE_DOTS, PICTURE};

/// The bytes of a teletext packet.
const PACKET_BYTES: usize = neoteletext_core::consts::PACKET_BYTES;
/// The Fastext keys a stored X/27/0 carries: links 0 to 5 (EN 300 706
/// clause 9.6.1, Table 15), as the engine's page writer copies them.
const FASTEXT_KEYS: usize = 6;
/// Whether a column-address mode of an X/26 triplet writes a character
/// into the cell at the active position (EN 300 706 clause 12.3.1, Table
/// 29): a G1 block mosaic, a G3 character at Level 1.5 or 2.5, a G0
/// character at Level 2.5, a DRCS character, a G2 character, or a G0
/// character with or without a diacritical mark (modes 1xxxx).
const fn writes_character(mode: u8) -> bool {
    matches!(mode, MODE_G1 | MODE_G3_LEVEL_15 | MODE_G0_LEVEL_25 | MODE_G3_LEVEL_25 | MODE_DRCS_CHARACTER | MODE_G2)
        || mode & !MODE_DIACRITIC_MASK == MODE_G0_NO_DIACRITIC
}
/// The mode a blanked triplet takes: a G0 character with no diacritical
/// mark, whose data is then a space.
const BLANK_MODE: u8 = MODE_G0_NO_DIACRITIC;

/// A snapshot drawn: the sub-page drawn, its image, and its blanked
/// source.
pub struct Snapshot {
    /// The sub-code drawn, never 3F7F.
    pub sub: u16,
    /// Its image.
    pub image: SnapshotImage,
    /// Its source: its packets, then its picture's records as packets 4/31.
    pub source: Vec<u8>,
}

impl Engine {
    /// Draws the snapshot of `page`'s sub-page `sub` (3F7F: its first in
    /// ascending order, DEC-1526) from what this engine was fed, with
    /// reveal off, flash steady and no animation, and gives it with its
    /// blanked source, its packets, to which the caller adds its picture's
    /// records (REQ-6885 items 2 to 4).
    ///
    /// # Errors
    /// The refusal: no complete sub-page of the page, a picture that
    /// faulted or stopped at its work allowance, a source that could not
    /// be blanked and checked, or one past `MAX_SNAPSHOT_BYTES`.
    pub fn snapshot(&mut self, page: u16, sub: u16) -> Result<Snapshot, SnapshotRefusal> {
        let address = PageAddress::from_mpp(page).ok_or(SnapshotRefusal::NoSuchSubPage)?;
        let store = self.decoder.store();
        let subcode = if sub == SUBCODE_ANY { store.first_subpage(address).ok_or(SnapshotRefusal::NoSuchSubPage)? } else { SubCode::new(sub) };
        let stored = *store.get(address, subcode).filter(|stored| stored.state == PageState::Complete).ok_or(SnapshotRefusal::NoSuchSubPage)?;
        let drawn = subcode.value();
        // Reveal off whatever any view says, steady flash, no gradual
        // painting: the still as the page area would show it at rest.
        self.set_reduced_motion(true);
        self.set_paint_gradually(false);
        let view = View { page, subpage: Some(drawn), ..View::default() };
        self.set_view(view);
        let original = self.compose(page, &stored, view);
        let blanked = blank_concealed(&stored, &original, |candidate| self.compose(page, candidate, view));
        let check = self.compose(page, &blanked, view);
        if render(&check, self.config()) != render(&original, self.config()) || !shows_nothing_hidden(&check, &original) {
            return Err(SnapshotRefusal::Rejected);
        }
        let image = match self.take_pictures(page, Some(drawn), self.now_us()) {
            (Some(regis), _) | (None, Some(regis)) => SnapshotImage::Regis(regis.picture),
            (None, None) => {
                let (palette, screen, indices) = render(&original, self.config());
                SnapshotImage::Frame { palette, screen, indices }
            }
        };
        if self.naplps.take_notes().iter().any(|note| note.page == page && note.sub == drawn) {
            return Err(SnapshotRefusal::PictureFailed);
        }
        let mut source = Vec::new();
        neoteletext_formats::encode::encode_page(&page_file(&blanked), &mut |packet: Packet| source.extend_from_slice(&packet.bytes));
        if source.len() > MAX_SNAPSHOT_BYTES {
            return Err(SnapshotRefusal::TooLarge);
        }
        Ok(Snapshot { sub: drawn, image, source })
    }
}

/// A grid's image with reveal off at the steady flash phase, as the page
/// area draws it: its palette, its screen colour and its indices.
pub(super) fn render(grid: &Grid, config: &neoteletext_core::config::Config) -> (Vec<[u8; 3]>, u8, Vec<u8>) {
    let mut indices = vec![0; PAGE_DOTS];
    let options = RenderOptions::resolved(config, false, FlashPhase::STEADY).with_underline(false);
    let mut buffer = IndexBuffer { format: IndexFormat::Bits8, data: &mut indices, stride: usize::from(interface_core::canvas::PAGE_WIDTH) };
    // A grid the engine composed always renders; were it not to, the image
    // is left blank, and the check of the blanked page against it fails.
    let _ = render_screen(grid, ALL_ROWS, &GLYPHS, options, &mut buffer);
    let palette = colour_map_palette(&grid.colour_map, &grid.receiver_colours, PICTURE).iter().map(|colour| [colour.r, colour.g, colour.b]).collect();
    (palette, grid.screen_colour, indices)
}

/// Every glyph some cell shows with reveal off: those of the cells not
/// concealed.
fn shown_glyphs(grid: &Grid) -> Vec<Glyph> {
    let mut shown: Vec<Glyph> = Vec::new();
    for cell in grid.cells.iter().flatten() {
        if !cell.conceal && !cell.glyph.is_blank() && !shown.contains(&cell.glyph) {
            shown.push(cell.glyph);
        }
    }
    shown
}

/// REQ-6885 item 4's second property: with every concealed cell of
/// `blanked` revealed, no cell shows a glyph that no cell of `original`
/// shows with reveal off.
fn shows_nothing_hidden(blanked: &Grid, original: &Grid) -> bool {
    let shown = shown_glyphs(original);
    blanked.cells.iter().flatten().all(|cell| cell.glyph.is_blank() || shown.contains(&cell.glyph))
}

/// `stored` with every concealed character that no cell of `original`
/// shows blanked: a Level 1 code under a concealed cell becomes a space,
/// and a triplet whose character lands in such a cell writes a space.
/// `compose` composes a candidate as the page is drawn.
fn blank_concealed(stored: &StoredPage, original: &Grid, compose: impl Fn(&StoredPage) -> Grid) -> StoredPage {
    let shown = shown_glyphs(original);
    let hidden = |glyph: Glyph| !glyph.is_blank() && !shown.contains(&glyph);
    let mut blanked = *stored;
    // The enhancement triplets first, each tried alone against the page as
    // it stands: a space in its place shows which cells it writes.
    for designation in 0..blanked.enhancements.len() {
        let Some(triplets) = blanked.enhancements.get(designation).copied() else { continue };
        for (index, triplet) in triplets.iter().enumerate() {
            let Some(triplet) = triplet.filter(|triplet| triplet.address < ROW_ADDRESS_BASE && writes_character(triplet.mode)) else {
                continue;
            };
            let mut candidate = blanked;
            if let Some(slot) = candidate.enhancements.get_mut(designation).and_then(|row| row.get_mut(index)) {
                *slot = Some(neoteletext_core::packet::Triplet { mode: BLANK_MODE, data: SPACE, ..triplet });
            }
            let tried = compose(&candidate);
            let writes_hidden = original
                .cells
                .iter()
                .flatten()
                .zip(tried.cells.iter().flatten())
                .any(|(before, after)| before.glyph != after.glyph && before.conceal && hidden(before.glyph));
            if writes_hidden {
                blanked = candidate;
            }
        }
    }
    // Then the Level 1 codes under concealed cells, whose glyph at Level 1
    // is read from a grid of the page with no enhancement at all.
    let level1 = level1_glyphs(stored, &compose);
    for (row, cells) in original.cells.iter().enumerate() {
        for (column, cell) in cells.iter().enumerate() {
            if !cell.conceal {
                continue;
            }
            let Some(code) = blanked.cells.get(row).and_then(|codes| codes.get(column)).map(|&byte| byte & CODE_MASK_7BIT) else { continue };
            // Spacing attributes stay, so that the layout does not move.
            if code < SPACE {
                continue;
            }
            let glyph = level1.get(row).and_then(|glyphs| glyphs.get(column)).copied();
            if glyph.is_some_and(hidden) {
                set_space(&mut blanked, row, column);
            }
        }
    }
    blanked
}

/// The glyph each cell of `stored` shows at Level 1: the page composed
/// with its enhancement triplets taken away.
fn level1_glyphs(stored: &StoredPage, compose: &impl Fn(&StoredPage) -> Grid) -> Vec<Vec<Glyph>> {
    let mut plain = *stored;
    plain.enhancements_present = 0;
    plain.enhancements.fill([None; neoteletext_core::consts::TRIPLETS_PER_PACKET]);
    compose(&plain).cells.iter().map(|row| row.iter().map(|cell| cell.glyph).collect()).collect()
}

/// A space, which has odd parity as it is (0x20 has one bit set), in
/// `row` and `column`, marked as passing parity.
fn set_space(page: &mut StoredPage, row: usize, column: usize) {
    if let Some(byte) = page.cells.get_mut(row).and_then(|codes| codes.get_mut(column)) {
        *byte = SPACE;
    }
    let (Some(good), Ok(bit)) = (page.parity_ok.get_mut(row), u32::try_from(column)) else { return };
    *good |= 1_u64.checked_shl(bit).unwrap_or(0);
}

/// A page file of a stored sub-page, as the engine's page writer takes
/// one: its rows as 7-bit codes (a byte that failed parity as a space, the
/// engine's decision E.8), its control bits, its X/26 packets and its
/// X/27/0 links. TODO-6913 asks the engine for this as a function of its
/// own, which `neottx convert` has privately.
fn page_file(page: &StoredPage) -> PageFile {
    let mut file = PageFile { address: page.key.address, subcode: page.key.subcode, ..PageFile::default() };
    file.set_control(page.control);
    for row in 0..STORED_ROWS {
        let Ok(number) = u8::try_from(row) else { continue };
        if row > 0 && !page.has_row(number) {
            continue;
        }
        let mut codes = [SPACE; COLUMNS];
        let first = if row == 0 { HEADER_DISPLAY_COLUMN } else { 0 };
        for (column, code) in codes.iter_mut().enumerate().skip(first) {
            let (byte, passed) = page.byte(row, column, ParityPolicy::KeepLastGood);
            *code = if passed { byte & CODE_MASK_7BIT } else { SPACE };
        }
        file.set_row(row, &codes);
        file.rows_present |= 1_u32.checked_shl(u32::from(number)).unwrap_or(0);
    }
    for (designation, triplets) in page.enhancements.iter().enumerate() {
        let (Ok(code), Ok(bit)) = (u8::try_from(designation), u32::try_from(designation)) else { continue };
        if page.enhancements_present & 1_u16.checked_shl(bit).unwrap_or(0) != 0 {
            let mut packet = EnhancementPacket { designation: code, triplets: Default::default() };
            for (slot, triplet) in packet.triplets.iter_mut().zip(triplets) {
                *slot = triplet.unwrap_or_default();
            }
            let _ = file.enhancements.push(packet);
        }
    }
    if let Some(links) = page.links {
        for (slot, link) in file.fastext.iter_mut().zip(links.links.iter()).take(FASTEXT_KEYS) {
            *slot = link.and_then(|link| {
                let mpp = (u16::from(link.magazine.number()) << 8) | u16::from(link.page.0);
                PageAddress::from_mpp(mpp).map(|address| FastextLink { address, subcode: link.subcode })
            });
        }
        file.fastext_row_24 = links.display_row_24;
    }
    page_packets(page, &mut file);
    file
}

/// The packets X/27/4, X/27/5 and X/28 of a stored page, encoded again from
/// their decoded fields as the engine's page writer does: a link that was
/// uncorrectable is written as no link (page validity 00).
fn page_packets(page: &StoredPage, file: &mut PageFile) {
    use neoteletext_core::consts::{COMPOSITIONAL_LINKS, DESIGNATION_LEVEL_35_DATA, DESIGNATION_LINKS_LEVEL_25, DESIGNATION_PAGE_DATA, LINK_NULL_PAGE};
    use neoteletext_core::packet::{CompositionalLink, PageFormat, encode};
    use neoteletext_core::types::PageNumber;
    use neoteletext_formats::encode::packed_line_of;
    use neoteletext_formats::page::PageFunction;
    let push_line = |file: &mut PageFile, packet: &Packet| {
        if let Some(line) = packed_line_of(packet) {
            let _ = file.raw_packets.push(line);
        }
    };
    let magazine = page.key.address.magazine;
    for (designation, links) in (DESIGNATION_LINKS_LEVEL_25..).zip(page.compositional_links) {
        let Some(links) = links else { continue };
        let none =
            CompositionalLink { function: 0, validity: 0, magazine, page: PageNumber(LINK_NULL_PAGE), subpages: 0, marker_first: true, marker_second: false };
        let mut all = [none; COMPOSITIONAL_LINKS];
        for (slot, link) in all.iter_mut().zip(links) {
            *slot = link.unwrap_or(none);
        }
        push_line(file, &encode::compositional_links(magazine, designation, &all));
    }
    let basic = PageFormat { function: 0, coding: 0 };
    for (data, designation) in [(page.page_data, DESIGNATION_PAGE_DATA), (page.page_data_level35, DESIGNATION_LEVEL_35_DATA)] {
        let Some(data) = data else { continue };
        let format = data.format.unwrap_or(basic);
        push_line(file, &encode::page_enhancement(magazine, designation, format, &data.fields));
        file.function = Some(PageFunction { function: format.function, coding: format.coding });
    }
}

// The source's packets are whole.
const _: () = assert!(PACKET_BYTES == 42);
