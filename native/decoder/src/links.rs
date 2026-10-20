//! Finding the links of a page (`hyperlinks.md` section 6): page numbers
//! printed in its text, and the runs of text and page numbers its TTI page
//! file links (`LT` and `LK` lines). All of it is untrusted: the trusted
//! core checks every link again and decides where each leads.

use std::collections::BTreeMap;

use neoteletext_core::charset::to_unicode;
use neoteletext_core::consts::COLUMNS;
use neoteletext_core::level1::{Cell, Glyph, Grid, Size};
use neoteletext_formats::page::{LinkDestination, PageFile};
use protocol::links::{Candidate, Found, MAX_FOUND, MAX_NUMBER_LINKS, MAX_TARGET, MAX_TEXT_LINKS, NumberLink};

/// Text rows: row 0 is the header and row 24 the Fastext row.
const FIRST_ROW: usize = 1;
const LAST_ROW: usize = 23;
/// A page number is three digits, the first the magazine, 1 to 8
/// (EN 300 706 clause 7.1.2).
const PAGE_DIGITS: usize = 3;
const FIRST_MAGAZINE: char = '1';
const LAST_MAGAZINE: char = '8';
/// Signs before a number that make it an amount, not a page.
const CURRENCY: [char; 5] = ['£', '$', '€', '¥', '#'];
/// "p302": a page number may follow a lone `p`.
const PAGE_PREFIX: char = 'p';
/// Separators that, between digits, make a number part of a larger one
/// ("3.142", "1,500", "12:30").
const SEPARATORS: [char; 3] = ['.', ',', ':'];
/// After a number, these make it a quantity.
const QUANTITY: char = '%';
/// Light backgrounds take a black edge on the underline: those brighter
/// than half, by the luminance of ITU-R BT.709, the weights the engine's
/// colours are defined against.
const LUMA_RED: f32 = 0.2126;
const LUMA_GREEN: f32 = 0.7152;
const LUMA_BLUE: f32 = 0.0722;
const LIGHT: f32 = 0.5 * 255.0;
const DIGIT_SHIFT: u32 = 4;

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

/// The character a cell shows as text, if it shows one: not a mosaic, not
/// concealed, and not the lower half of a double-height character, whose
/// upper half is read instead.
fn character(cell: &Cell) -> Option<char> {
    if cell.conceal || cell.size == Size::DoubleBottom {
        return None;
    }
    match cell.glyph {
        Glyph::Char(character) => Some(to_unicode(character).base),
        Glyph::Space => Some(' '),
        Glyph::Mosaic { .. } => None,
    }
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

/// The page numbers printed in a row of text (`hyperlinks.md` section
/// 6.1): three digits standing alone, the first 1 to 8, perhaps after a
/// lone `p`; not an amount, a quantity, a time or part of a longer number.
fn numbers_in(row: &[Option<char>]) -> Vec<(usize, u16)> {
    let at = |column: Option<usize>| column.and_then(|column| row.get(column).copied().flatten());
    let digit = |column: Option<usize>| at(column).is_some_and(|character| character.is_ascii_digit());
    let word = |column: Option<usize>| at(column).is_some_and(char::is_alphanumeric);
    let mut found = Vec::new();
    let mut column = 0;
    while column < row.len() {
        let length = (column..row.len()).take_while(|&index| digit(Some(index))).count();
        if length == 0 {
            column = column.saturating_add(1);
            continue;
        }
        let start = column;
        column = column.saturating_add(length);
        let end = start.saturating_add(length);
        let first = at(Some(start)).unwrap_or(' ');
        if length != PAGE_DIGITS || !(FIRST_MAGAZINE..=LAST_MAGAZINE).contains(&first) {
            continue;
        }
        let before = start.checked_sub(1);
        let before_that = start.checked_sub(2);
        let after = Some(end);
        let after_that = end.checked_add(1);
        let prefixed = at(before).is_some_and(|character| character.eq_ignore_ascii_case(&PAGE_PREFIX)) && !word(before_that);
        let joined_before = at(before).is_some_and(|character| SEPARATORS.contains(&character)) && digit(before_that);
        let joined_after = at(after).is_some_and(|character| SEPARATORS.contains(&character)) && digit(after_that);
        let amount = at(before).is_some_and(|character| CURRENCY.contains(&character));
        let quantity = at(after) == Some(QUANTITY);
        if (word(before) && !prefixed) || word(after) || joined_before || joined_after || amount || quantity {
            continue;
        }
        let page = (start..end).filter_map(|index| at(Some(index))?.to_digit(10)).fold(0_u32, |page, digit| (page << DIGIT_SHIFT) | digit);
        if let Ok(page) = u16::try_from(page) {
            found.push((start, page));
        }
    }
    found
}

/// The links of a page: the runs its `LT` lines declare, trimmed to the
/// cells that draw something, then the page numbers in its text, at most
/// 24; and its `LK` lines.
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
            target: Found::Declared(target.clone()),
        });
    }
    let mut numbers = 0;
    for row in FIRST_ROW..=LAST_ROW {
        let text: Vec<Option<char>> = (0..COLUMNS).map(|column| grid.cell(row, column).and_then(character)).collect();
        for (column, page) in numbers_in(&text) {
            if numbers == MAX_FOUND {
                break;
            }
            let Some(cell) = grid.cell(row, column) else { continue };
            let (Ok(row), Ok(column)) = (u8::try_from(row), u8::try_from(column)) else { continue };
            numbers = numbers.saturating_add(1);
            candidates.push(Candidate {
                row,
                column,
                length: 3,
                double: cell.size == Size::DoubleTop,
                light: light(cell, palette),
                target: Found::Number(page),
            });
        }
    }
    (candidates, lines.map(|lines| lines.numbers.clone()).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(text: &str) -> Vec<Option<char>> {
        text.chars().map(Some).collect()
    }

    fn pages(text: &str) -> Vec<u16> {
        numbers_in(&row(text)).into_iter().map(|(_, page)| page).collect()
    }

    #[test]
    fn page_numbers_stand_alone() {
        assert_eq!(pages("Sport 140"), [0x140]);
        assert_eq!(pages("see p302 and P401"), [0x302, 0x401]);
        assert_eq!(pages("Results 140-145"), [0x140, 0x145], "a range links both ends");
        assert_eq!(pages("100 Index"), [0x100]);
        for text in ["£150", "$150", "#150", "150%", "12:30", "123:45", "3.142", "1,500", "1984", "40", "No.302x", "ap302", "a302", "902", "099"] {
            assert_eq!(pages(text), [] as [u16; 0], "{text:?} is not a page number");
        }
        let (column, _) = numbers_in(&row("Sport 140"))[0];
        assert_eq!(column, 6);
    }

    #[test]
    fn mosaics_and_concealed_cells_hold_no_digits() {
        let mut text = row("x140x");
        text[0] = None;
        text[4] = None;
        assert_eq!(numbers_in(&text).len(), 1, "a mosaic beside a number is not a letter");
        text[2] = None;
        assert!(numbers_in(&text).is_empty(), "a concealed digit breaks the number");
    }
}
