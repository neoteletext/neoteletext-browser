//! A page's title for the page list: the first text row with a few
//! letters or digits.

use neoteletext_core::charset::to_unicode;
use neoteletext_core::consts::COLUMNS;
use neoteletext_core::level1::{Glyph, Grid};

/// A title is the first text row with at least this many letters or digits.
const TITLE_MINIMUM: usize = 3;
/// A row is a picture, not a title, when more than a quarter of its drawn
/// cells are mosaics.
const MOSAIC_SHARE: usize = 3;
/// Row 0 is the header; text rows are 1 to 23, row 24 the Fastext row.
const FIRST_TEXT_ROW: usize = 1;
const LAST_TEXT_ROW: usize = 23;

/// The first text row with a few letters or digits: a title for the page
/// list. The engine's grid says which cells are text, as opposed to
/// mosaics, and its character sets give each text cell's character, with
/// the page's national characters. Concealed text is left out.
pub(crate) fn title(grid: &Grid) -> String {
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
