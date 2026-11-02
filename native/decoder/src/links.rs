//! Finding the links of a page (`hyperlinks.md` section 6): the runs of
//! text and the page numbers its TTI page file links (`LT` and `LK`
//! lines). Only what the page declares is a link: a page number printed
//! in its text is just text (decision 116). All of it is untrusted: the
//! trusted core checks every link again and decides where each leads.

use std::collections::BTreeMap;

use neoteletext_core::charset::to_unicode;
use neoteletext_core::level1::{Cell, Glyph, Grid, Size};
use neoteletext_formats::page::{AddressSpan, LINK_LINES, LINK_TARGET_BYTES, LinkDestination, LinkLine, LinkText, NUMBER_LINKS, PageFile, TEXT_LINKS};
use protocol::links::{Candidate, MAX_NUMBER_LINKS, MAX_TARGET, MAX_TEXT_LINKS, NumberLink};

// A target the engine keeps is one a message can carry.
const _: () = assert!(MAX_TARGET == LINK_TARGET_BYTES, "the protocol's target is the engine's");

#[cfg(test)]
mod tests;

/// Light backgrounds take a black edge on the underline: those brighter
/// than half, by the luminance of ITU-R BT.709, the weights the engine's
/// colours are defined against.
const LUMA_RED: f32 = 0.2126;
const LUMA_GREEN: f32 = 0.7152;
const LUMA_BLUE: f32 = 0.0722;
const LIGHT: f32 = 0.5 * 255.0;

/// The `LT` and `LK` lines of one page file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkLines {
    /// `LT`: row, column, length and target.
    text: Vec<(u8, u8, u8, String)>,
    /// `LK`: a page number and its target.
    numbers: Vec<NumberLink>,
    /// A LINKS record's `LN` lines. A page file's stay in the page file,
    /// which draws its row 24 as it is read.
    keys: Vec<LinkLine>,
    /// The addresses of `keys`, which outlast the record they came in.
    key_text: LinkText,
}

impl LinkLines {
    /// The receiver's row 24 and keys these lines give, as a page file
    /// with the same `LN` lines gives them (the engine's REQ-NAV-011).
    #[must_use]
    pub fn key_file(&self) -> Option<PageFile> {
        if self.keys.is_empty() {
            return None;
        }
        let mut file = PageFile::new();
        for &line in &self.keys {
            file.links.push(line);
        }
        file.link_text = self.key_text.clone();
        Some(file)
    }
}

/// The link lines of every page file read, by page number and sub-code,
/// as the store keeps pages.
#[derive(Default)]
pub struct Lines {
    held: BTreeMap<(u16, u16), LinkLines>,
    /// Each pushed sub-page's LINKS sending so far.
    sendings: BTreeMap<(u16, u16), Sending>,
}

/// A sub-page's LINKS sending as its parts arrive: how many parts it has,
/// the part that comes next, and the links its parts gave, against the
/// totals of a whole sending (DOC-1958).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Sending {
    parts: u8,
    next: u8,
    /// The keys given, each at most once: four coloured and Index.
    keys: Vec<neoteletext_formats::page::LinkKey>,
    /// The page numbers given, each at most once.
    numbers: Vec<u16>,
    /// How many text links were given.
    texts: usize,
}

impl Sending {
    /// Takes `record`'s links into the sending's totals; false when a key or
    /// a page number comes a second time, or a kind passes its bound over
    /// the whole sending.
    fn take(&mut self, record: &neoteletext_web::records::links::LinksRecord<'_>) -> bool {
        for line in record.key_links() {
            if self.keys.contains(&line.key) || self.keys.len() >= LINK_LINES {
                return false;
            }
            self.keys.push(line.key);
        }
        for link in record.number_links() {
            let number = link.number.mpp();
            if self.numbers.contains(&number) || self.numbers.len() >= NUMBER_LINKS {
                return false;
            }
            self.numbers.push(number);
        }
        self.texts = self.texts.saturating_add(record.text_links().count());
        self.texts <= TEXT_LINKS
    }
}

impl Lines {
    /// Keeps the link lines of a page file, replacing its earlier version's.
    pub fn read(&mut self, page: &PageFile) {
        let key = (page.address.mpp(), page.subcode.value());
        let (texts, numbers) = (page.text_links.as_slice().iter().copied(), page.number_links.as_slice().iter().copied());
        self.set(key, texts, numbers, core::iter::empty(), |span| page.link_text.address(span), true);
    }

    /// Keeps a pushed sub-page's links from its LINKS record (REQ-1494),
    /// as its page file's `LT`, `LK` and `LN` lines would give them: part
    /// 0 replaces what was held, later parts add to it. A later part is
    /// taken only as the next of the sending part 0 began, with the same
    /// number of parts, and while the sending keeps a whole sending's totals:
    /// each key and each page number once, at most five key links, eight
    /// page-number links and eight text links over all its parts. A part
    /// that breaks either is refused, and a LINKS record refused leaves its
    /// sub-page with no Link Extension links (DOC-1952, DOC-1958). The
    /// engine's reader checked each record alone.
    pub fn read_record(&mut self, record: &neoteletext_web::records::links::LinksRecord<'_>) {
        let key = (record.page.mpp(), record.subcode.value());
        let mut sending = if record.part == 0 {
            Sending { parts: record.parts, ..Sending::default() }
        } else {
            match self.sendings.get(&key) {
                Some(sending) if (sending.parts, sending.next) == (record.parts, record.part) => sending.clone(),
                _ => {
                    self.clear(key.0, key.1);
                    return;
                }
            }
        };
        if !sending.take(record) {
            self.clear(key.0, key.1);
            return;
        }
        sending.next = record.part.saturating_add(1);
        self.sendings.insert(key, sending);
        self.set(key, record.text_links(), record.number_links(), record.key_links(), |span| record.address(span), record.part == 0);
    }

    /// Drops what is held for a sub-page a frame came for with no LINKS
    /// record: it has no links (neoteletext-common's DEC-1517).
    pub fn clear(&mut self, page: u16, subcode: u16) {
        self.held.remove(&(page, subcode));
        self.sendings.remove(&(page, subcode));
    }

    fn set<'a>(
        &mut self,
        key: (u16, u16),
        texts: impl Iterator<Item = neoteletext_formats::page::TextLink>,
        numbers: impl Iterator<Item = neoteletext_formats::page::NumberLink>,
        keys: impl Iterator<Item = LinkLine>,
        address: impl Fn(AddressSpan) -> &'a [u8],
        replace: bool,
    ) {
        let mut lines = if replace { LinkLines::default() } else { self.held.remove(&key).unwrap_or_default() };
        lines.text.extend(texts.filter_map(|link| Some((link.row, link.column, link.length, target(&link.destination, &address)?))));
        lines.text.truncate(MAX_TEXT_LINKS);
        lines.numbers.extend(numbers.filter_map(|link| Some(NumberLink { number: link.number.mpp(), target: target(&link.destination, &address)? })));
        lines.numbers.truncate(MAX_NUMBER_LINKS);
        for mut line in keys.take(LINK_LINES.saturating_sub(lines.keys.len())) {
            // A key's address leaves its record for the text the key keeps.
            let kept = if line.url.is_empty() { Some(AddressSpan::NONE) } else { lines.key_text.add(address(line.url)) };
            if let Some(span) = kept {
                line.url = span;
                lines.keys.push(line);
            }
        }
        if lines == LinkLines::default() {
            self.held.remove(&key);
        } else {
            self.held.insert(key, lines);
        }
    }

    /// The link lines of a page as stored.
    #[must_use]
    pub fn get(&self, page: u16, subcode: u16) -> Option<&LinkLines> {
        self.held.get(&(page, subcode))
    }
}

/// A target as the page file wrote it (TTI Link Extension section E.1):
/// an address, `web:` included, as written; otherwise the page, after the
/// service's name if it has one. `address` reads what holds the address.
/// The trusted core parses it again.
fn target<'a>(destination: &LinkDestination, address: &impl Fn(AddressSpan) -> &'a [u8]) -> Option<String> {
    let text = if destination.url.is_empty() {
        let page = destination.address.mpp();
        let service = String::from_utf8_lossy(destination.service.as_bytes());
        if service.is_empty() { format!("{page:03X}") } else { format!("{service}:{page:03X}") }
    } else {
        String::from_utf8_lossy(address(destination.url)).into_owned()
    };
    (text.len() <= MAX_TARGET).then_some(text)
}

/// Whether a cell draws something: a character other than a space, or a
/// mosaic with a dot, and not concealed.
fn drawn(cell: &Cell) -> bool {
    !cell.conceal
        && match cell.glyph {
            Glyph::Char(character) => !to_unicode(character).base.is_whitespace(),
            Glyph::Mosaic { pattern, .. } => pattern != 0,
            Glyph::Space => false,
        }
}

/// Whether a cell's background is light, by the page's own colours.
fn light(cell: &Cell, palette: &[[u8; 3]]) -> bool {
    palette
        .get(usize::from(cell.background))
        .is_some_and(|&[red, green, blue]| LUMA_RED * f32::from(red) + LUMA_GREEN * f32::from(green) + LUMA_BLUE * f32::from(blue) > LIGHT)
}

/// The links of a page: the runs its `LT` lines declare, trimmed to the
/// cells that draw something; and its `LK` lines.
#[must_use]
pub fn find(grid: &Grid, palette: &[[u8; 3]], lines: Option<&LinkLines>) -> (Vec<Candidate>, Vec<NumberLink>) {
    let mut candidates = Vec::new();
    for (row, column, length, target) in lines.map(|lines| lines.text.as_slice()).unwrap_or_default() {
        let cells: Vec<(usize, &Cell)> = (usize::from(*column)..usize::from(*column).saturating_add(usize::from(*length)))
            .filter_map(|index| Some((index, grid.cell(usize::from(*row), index)?)))
            .collect();
        let shown = |(_, cell): &&(usize, &Cell)| drawn(cell);
        let (Some(&(first, first_cell)), Some(&(last, _))) = (cells.iter().find(shown), cells.iter().rev().find(shown)) else { continue };
        let (Ok(start), Ok(length)) = (u8::try_from(first), u8::try_from(last.saturating_sub(first).saturating_add(1))) else { continue };
        candidates.push(Candidate {
            row: *row,
            column: start,
            length,
            double: first_cell.size == Size::DoubleTop,
            light: light(first_cell, palette),
            target: target.clone(),
        });
    }
    (candidates, lines.map(|lines| lines.numbers.clone()).unwrap_or_default())
}
