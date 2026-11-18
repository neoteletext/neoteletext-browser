//! The sub-pages a channel draws in NAPLPS (the NAPLPS renderer's
//! REQ-6408 item 5, DEC-6407): for each, a NAPLPS terminal fed the
//! sub-page's records in order, and its picture, sent when it changed as
//! a ReGIS picture is.
//!
//! NAPLPS (ANSI X3.110-1983) draws on a unit screen 1 wide and 0.75 high,
//! which 640 by 480 pixels hold exactly. The picture the browser's
//! processes carry is 800 by 480, so the NAPLPS canvas lies in its middle
//! 640 columns and the columns either side take the terminal's border
//! colour. WAIT is not waited for and BLINK is not animated: the colour
//! map is shown as it stands.

use std::collections::BTreeMap;

use naplps::{Canvas, StrokeFont, Terminal};
use neoteletext_web::records::regis::RegisRecord;
use protocol::regis::{PictureLanguage, REGIS_COLOURS, REGIS_HEIGHT, REGIS_WIDTH, RegisPick};

use crate::parts::{Figures, Sent, parts_to_send};
use crate::regis::Drawn;

/// The colour map's address bits: sixteen entries, the picture's colours.
const PALETTE_BITS: u32 = 4;
/// The canvas: the unit screen, 1 by 0.75, with square pixels.
pub(crate) const WIDTH: usize = 640;
pub(crate) const HEIGHT: usize = REGIS_HEIGHT;
/// The picture's columns to the left of the canvas, which lies in its middle.
const LEFT: usize = (REGIS_WIDTH - WIDTH) / 2;
/// The most sub-pages whose NAPLPS is kept, as for ReGIS: one more, and
/// the one fed longest ago is forgotten.
pub const MAX_NAPLPS_PAGES: usize = crate::regis::MAX_REGIS_PAGES;

const _: () = assert!(WIDTH <= REGIS_WIDTH && (1 << PALETTE_BITS) == REGIS_COLOURS);

/// One sub-page's NAPLPS.
struct Page {
    /// Which stream of the sub-page is being drawn.
    generation: u16,
    /// The offset the next record's bytes begin at.
    next: u32,
    /// A record was lost: nothing more of this generation is drawn.
    lost: bool,
    terminal: Terminal,
    canvas: Canvas,
    /// Bytes were drawn since the picture was last sent.
    changed: bool,
    /// When it was last fed, by the count of records read.
    fed: u64,
    /// The records and the bytes drawn of this stream, which the browser
    /// shows above the picture.
    records: u32,
    bytes: u64,
    /// The picture last sent, which the next is sent as parts of.
    sent: Option<Sent>,
}

impl Page {
    /// A sub-page's NAPLPS begun; `None` only if the canvas cannot be made.
    fn new(generation: u16) -> Option<Self> {
        let mut terminal = Terminal::new(PALETTE_BITS);
        // The font is this program's own data: if it did not parse, text
        // would move the cursor and draw nothing.
        if let Ok(font) = StrokeFont::parse(naplps_fonts::NEOTELETEXT) {
            terminal.set_font(font);
        }
        let canvas = Canvas::new(u32::try_from(WIDTH).ok()?, u32::try_from(HEIGHT).ok()?).ok()?;
        Some(Self { generation, next: 0, lost: false, terminal, canvas, changed: true, fed: 0, records: 0, bytes: 0, sent: None })
    }

    /// The picture as the browser's processes carry it, one byte a pixel:
    /// the canvas in the middle columns, the border's colour either side.
    fn pixels(&self) -> Vec<u8> {
        let mut pixels = vec![self.terminal.border(); REGIS_WIDTH.saturating_mul(REGIS_HEIGHT)];
        for (row, line) in pixels.as_chunks_mut::<REGIS_WIDTH>().0.iter_mut().zip(self.canvas.pixels().as_chunks::<WIDTH>().0) {
            if let Some(middle) = row.get_mut(LEFT..LEFT.saturating_add(WIDTH)) {
                middle.copy_from_slice(line);
            }
        }
        pixels
    }

    /// The colour map's sixteen colours.
    fn palette(&self) -> [[u8; 3]; REGIS_COLOURS] {
        let mut palette = [[0_u8; 3]; REGIS_COLOURS];
        for (colour, entry) in palette.iter_mut().zip(self.terminal.palette().colours()) {
            *colour = *entry;
        }
        palette
    }
}

/// Every sub-page's NAPLPS a channel has sent.
#[derive(Default)]
pub struct NaplpsPages {
    pages: BTreeMap<(u16, u16), Page>,
    /// Records read.
    records: u64,
    /// The sub-page whose picture was last sent for each page.
    sent_sub: BTreeMap<u16, u16>,
}

impl NaplpsPages {
    /// Reads one NAPLPS record: its bytes go to its sub-page's terminal.
    pub fn read(&mut self, record: &RegisRecord<'_>) {
        let key = (record.page.mpp(), record.subcode.value());
        self.records = self.records.saturating_add(1);
        let fresh = self.pages.get(&key).is_none_or(|page| page.generation != record.generation);
        if fresh {
            // A stream is followed from its first byte: one joined part
            // way through waits for the next generation.
            if record.offset != 0 {
                if let Some(page) = self.pages.get_mut(&key) {
                    page.lost = true;
                }
                return;
            }
            if !self.pages.contains_key(&key) && self.pages.len() >= MAX_NAPLPS_PAGES {
                let oldest = self.pages.iter().min_by_key(|(_, page)| page.fed).map(|(key, _)| *key);
                if let Some(oldest) = oldest {
                    self.pages.remove(&oldest);
                }
            }
            let Some(page) = Page::new(record.generation) else { return };
            self.pages.insert(key, page);
        }
        let Some(page) = self.pages.get_mut(&key) else { return };
        page.fed = self.records;
        if page.lost || record.offset != page.next {
            page.lost = true;
            return;
        }
        page.next = page.next.saturating_add(u32::try_from(record.data.len()).unwrap_or(u32::MAX));
        // What the terminal asks of its caller is not done here: WAIT is
        // not waited for, and a bell is not rung (DEC-6407 item 4).
        let _ = page.terminal.feed(&mut page.canvas, record.data);
        page.records = page.records.saturating_add(1);
        page.bytes = page.bytes.saturating_add(u64::try_from(record.data.len()).unwrap_or(u64::MAX));
        page.changed = true;
    }

    /// Whether a page has a sub-page drawn in NAPLPS.
    #[must_use]
    pub fn has(&self, page: u16) -> bool {
        self.pages.keys().any(|(held, _)| *held == page)
    }

    /// Marks a page's pictures as not yet sent: the page came on screen.
    pub fn show_again(&mut self, page: u16) {
        for (_, held) in self.pages.iter_mut().filter(|((held, _), _)| *held == page) {
            held.changed = true;
            held.sent = None;
        }
        self.sent_sub.remove(&page);
    }

    /// The picture of a page's sub-page on screen, if it changed since it
    /// was last taken: `sub`, or with none the page's first with any
    /// NAPLPS. Whole at first, and afterwards the areas that changed
    /// (REQ-6399). It names no drawings: NAPLPS has none.
    pub fn take(&mut self, page: u16, sub: Option<u16>) -> Option<Drawn> {
        let key = match sub {
            Some(sub) if sub != neoteletext_core::limits::SUBCODE_ANY => (page, sub),
            _ => self.pages.keys().find(|(held, _)| *held == page).copied()?,
        };
        let same_sub = self.sent_sub.get(&key.0) == Some(&key.1);
        let held = self.pages.get_mut(&key)?;
        if !held.changed {
            return None;
        }
        held.changed = false;
        let (pixels, palette) = (held.pixels(), held.palette());
        let figures = Figures { commits: held.records, work: held.bytes, language: PictureLanguage::Naplps };
        let pick = RegisPick::default();
        let (mut parts, pick_changed) = parts_to_send(&mut held.sent, same_sub, &pixels, palette, &pick, figures)?;
        self.sent_sub.insert(key.0, key.1);
        let more = parts.split_off(1);
        Some(Drawn { page: key.0, sub: key.1, picture: parts.pop()?, pick: pick_changed.then_some(pick), more })
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects,
        reason = "a test reports a failure by panicking, and indexes and counts what it made"
    )]

    use neoteletext_core::types::{PageAddress, SubCode};
    use protocol::regis::{RegisArea, unpack};

    use super::*;

    fn record(page: u16, generation: u16, offset: u32, data: &[u8]) -> RegisRecord<'_> {
        RegisRecord { page: PageAddress::from_mpp(page).unwrap(), subcode: SubCode::new(0), generation, offset, data }
    }

    /// REQ-6408 item 5: a sub-page's records draw a picture whose middle
    /// 640 columns are the NAPLPS canvas, with text drawn in the font
    /// built in, and whose other columns are the border's colour; it is
    /// NAPLPS's, with no pick record's drawings; more bytes are sent as
    /// the parts that changed, and a new generation starts again.
    #[test]
    fn records_draw_a_picture_in_the_middle_of_the_wider_one() {
        let mut pages = NaplpsPages::default();
        assert!(!pages.has(0x100));
        // Text in the primary set is its ASCII bytes, drawn from the home position.
        pages.read(&record(0x100, 1, 0, b"Hello"));
        assert!(pages.has(0x100));
        let drawn = pages.take(0x100, Some(0)).unwrap();
        assert!(drawn.picture.whole() && drawn.picture.language == PictureLanguage::Naplps);
        assert_eq!(drawn.pick, Some(RegisPick::default()), "no drawing is named anywhere");
        assert_eq!((drawn.picture.commits, drawn.picture.work), (1, 5), "a record and five bytes");
        let pixels = unpack(&drawn.picture.packed);
        let border = pixels[0];
        let column = |x: usize| (0..REGIS_HEIGHT).map(|y| pixels[y * REGIS_WIDTH + x]).collect::<Vec<u8>>();
        assert!((0..LEFT).chain(LEFT + WIDTH..REGIS_WIDTH).all(|x| column(x).iter().all(|&pixel| pixel == border)), "the columns either side are the border's");
        let inked = (0..REGIS_HEIGHT).flat_map(|y| (LEFT..LEFT + WIDTH).map(move |x| (x, y))).filter(|&(x, y)| pixels[y * REGIS_WIDTH + x] != border).count();
        assert!(inked > 50, "the text is drawn, in the font built in: {inked} pixels");
        assert!(pages.take(0x100, Some(0)).is_none(), "nothing changed since");

        pages.read(&record(0x100, 1, 5, b" there"));
        let more = pages.take(0x100, Some(0)).unwrap();
        assert!(!more.picture.whole() && more.pick.is_none(), "a part, and the pick record as before");
        assert!(more.picture.area != RegisArea::default() && more.picture.area.pixels() < REGIS_WIDTH * REGIS_HEIGHT / 10, "{:?}", more.picture.area);
        assert_eq!((more.picture.commits, more.picture.work), (2, 11));

        pages.read(&record(0x100, 1, 99, b"lost"));
        assert!(pages.take(0x100, Some(0)).is_none(), "a record after a gap is not drawn");
        pages.read(&record(0x100, 2, 0, b"Again"));
        let again = pages.take(0x100, Some(0)).unwrap();
        assert_eq!((again.picture.commits, again.picture.work), (1, 5), "a new generation starts the picture again");
    }

    /// As for ReGIS: one sub-page more than are kept, and the one fed
    /// longest ago is forgotten; a page that comes on screen again is
    /// sent whole.
    #[test]
    fn the_sub_page_fed_longest_ago_is_forgotten_and_a_page_shown_again_is_sent_whole() {
        let mut pages = NaplpsPages::default();
        for page in 0..=u16::try_from(MAX_NAPLPS_PAGES).unwrap() {
            pages.read(&record(0x100 + page, 1, 0, b"A"));
        }
        assert!(!pages.has(0x100), "the first fed is forgotten");
        assert!(pages.has(0x101));
        assert!(pages.take(0x101, None).unwrap().picture.whole());
        pages.read(&record(0x101, 1, 1, b"B"));
        pages.show_again(0x101);
        assert!(pages.take(0x101, None).unwrap().picture.whole(), "the page came on screen again");
    }
}
