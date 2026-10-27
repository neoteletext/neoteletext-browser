//! Finding the links of a page (`hyperlinks.md` section 6): the runs of
//! text and the page numbers its TTI page file links (`LT` and `LK`
//! lines). Only what the page declares is a link: a page number printed
//! in its text is just text (decision 116). All of it is untrusted: the
//! trusted core checks every link again and decides where each leads.

use std::collections::BTreeMap;

use neoteletext_core::charset::to_unicode;
use neoteletext_core::level1::{Cell, Glyph, Grid, Size};
use neoteletext_formats::page::{LinkDestination, PageFile};
use protocol::links::{Candidate, MAX_NUMBER_LINKS, MAX_TARGET, MAX_TEXT_LINKS, NumberLink};

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
}

/// The link lines of every page file read, by page number and sub-code,
/// as the store keeps pages.
#[derive(Default)]
pub struct Lines(BTreeMap<(u16, u16), LinkLines>);

impl Lines {
    /// Keeps the link lines of a page file, replacing its earlier version's.
    pub fn read(&mut self, page: &PageFile) {
        let key = (page.address.mpp(), page.subcode.value());
        let text = page
            .text_links
            .as_slice()
            .iter()
            .filter_map(|link| Some((link.row, link.column, link.length, target(&link.destination)?)))
            .take(MAX_TEXT_LINKS)
            .collect();
        let numbers = page
            .number_links
            .as_slice()
            .iter()
            .filter_map(|link| Some(NumberLink { number: link.number.mpp(), target: target(&link.destination)? }))
            .take(MAX_NUMBER_LINKS)
            .collect();
        let lines = LinkLines { text, numbers };
        if lines == LinkLines::default() {
            self.0.remove(&key);
        } else {
            self.0.insert(key, lines);
        }
    }

    /// The link lines of a page as stored.
    #[must_use]
    pub fn get(&self, page: u16, subcode: u16) -> Option<&LinkLines> {
        self.0.get(&(page, subcode))
    }
}

/// A target as the page file wrote it (TTI Link Extension section E.1):
/// an address, `web:` included, as written; otherwise the page, after the
/// service's name if it has one. The trusted core parses it again.
fn target(destination: &LinkDestination) -> Option<String> {
    let text = if destination.url.is_empty() {
        let page = destination.address.mpp();
        let service = String::from_utf8_lossy(destination.service.as_bytes());
        if service.is_empty() { format!("{page:03X}") } else { format!("{service}:{page:03X}") }
    } else {
        String::from_utf8_lossy(destination.url.as_bytes()).into_owned()
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
