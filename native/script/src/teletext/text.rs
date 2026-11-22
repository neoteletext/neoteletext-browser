//! Text through the engine's own encoder (script-api.md section 5.6;
//! DEC-2199): `neoteletext_formats::write_text::write_runs`, the path the
//! server encodes a channel program's text by, so that a script's text
//! chooses exactly the sets, marks and triplets a served page's does.
//! Nothing here knows a character set: the encoder is asked for one run
//! on an empty page, and what it wrote is read back cell by cell.
//!
//! A character the page's sets cannot show is refused by a `TypeError`
//! naming it, never replaced by another (the encoder's
//! `RunFault::Unshown`).

use std::fmt::Write as _;

use neoteletext_core::charset::language::language_value;
use neoteletext_core::consts::{COLUMNS, ESC, MODE_MODIFIED_DESIGNATION, MODE_ROW_SET, ROW_ADDRESS_BASE, ROWS, SPACE};
use neoteletext_core::packet::Triplet;
use neoteletext_formats::write_text::{Direction, LATIN_ENGLISH_TAG, Language, Level, PageText, RunFault, TextRun, languages, write_runs};

use super::limits::MAX_TEXT_CHARS;
use super::model::{OverlayCharacter, PlaneCell};
use crate::host::Reply;

/// A designation no language has: Table 32's values are seven bits. Named
/// as the page's when overlay text is encoded, it makes every run one "in
/// another set than the page's", which the encoder writes for Level 3.5
/// and below as a triplet for each character and no code in the row: the
/// form an overlay cell holds, since it must not touch the plane.
pub(super) const NO_PAGE_SET: u8 = 0xFF;
/// The row the encoder is given a run on when only its cells are wanted.
const ANY_ROW: u8 = neoteletext_core::consts::PACKET_FIRST_ROW;

/// The languages the engine writes, as a script names them, and the page's.
pub struct Languages {
    list: Vec<Language>,
    /// The page's language: the manifest's, or Latin English when the
    /// engine has no set for it.
    page: String,
    /// The Table 32 value of the page's set.
    page_value: u8,
}

/// A `RangeError` naming `bound`.
fn range(bound: &str, detail: &str) -> Reply {
    Reply::Range(format!("{bound}: {detail}"))
}

impl Languages {
    /// The engine's languages, for a page whose manifest names `tag`.
    #[must_use]
    pub fn new(tag: &str) -> Self {
        let (page, page_value) = match language_value(tag, true) {
            Some(value) => (tag.to_owned(), value),
            None => (LATIN_ENGLISH_TAG.to_owned(), language_value(LATIN_ENGLISH_TAG, true).unwrap_or(0)),
        };
        Self { list: languages(), page, page_value }
    }

    /// The Table 32 value of the page's set.
    #[must_use]
    pub const fn page_value(&self) -> u8 {
        self.page_value
    }

    /// The bytes held.
    #[must_use]
    pub fn state_bytes(&self) -> usize {
        self.list.iter().fold(self.page.len(), |sum, language| sum.saturating_add(size_of::<Language>()).saturating_add(language.tag.len()))
    }

    /// The language a call names by its place in [`Self::json`]'s list;
    /// −1 is the page's.
    fn tag(&self, language: i32) -> Result<Option<&str>, Reply> {
        if language == -1 {
            return Ok(None);
        }
        usize::try_from(language)
            .ok()
            .and_then(|at| self.list.get(at))
            .map(|language| Some(language.tag.as_str()))
            .ok_or_else(|| Reply::Type("the language is not one of nt.teletext.languages()".to_owned()))
    }

    /// `nt.teletext.languages()`: each language's tag, English name and
    /// direction, as JSON text.
    #[must_use]
    pub fn json(&self) -> String {
        let mut out = String::from("[");
        for (at, language) in self.list.iter().enumerate() {
            if at > 0 {
                out.push(',');
            }
            let direction = match language.direction {
                Direction::Ltr => "ltr",
                Direction::Rtl => "rtl",
            };
            // Tags and names are the engine's own table's: letters, digits,
            // spaces, hyphens and brackets, none of which JSON escapes.
            let _ = write!(out, "{{\"tag\":\"{}\",\"name\":\"{}\",\"direction\":\"{direction}\"}}", language.tag, language.name);
        }
        out.push(']');
        out
    }

    /// One run through the encoder, on an otherwise empty page: the row
    /// as the encoder left it, and the column triplets it needs.
    fn encode(&self, run: TextRun<'_>, own: bool, before: &[PlaneCell]) -> Result<([u8; COLUMNS], Vec<Triplet>, usize), Reply> {
        if run.text.chars().count() > MAX_TEXT_CHARS {
            return Err(range("MAX_TEXT_CHARS", "a text is at most 1,024 characters"));
        }
        let mut grid = [[SPACE; COLUMNS]; ROWS];
        // The codes before the run stay as the page has them, so that the
        // encoder sees an ESC that would leave the run in the second set.
        if let Some(line) = grid.get_mut(usize::from(run.row)) {
            for (slot, cell) in line.iter_mut().zip(before) {
                *slot = cell.code;
            }
        }
        let runs = [run];
        let page = if own {
            PageText { language: Some(self.page.as_str()), designation: self.page_value, level: Level::NeoTeletext, runs: &runs }
        } else {
            PageText { language: None, designation: NO_PAGE_SET, level: Level::Standard, runs: &runs }
        };
        let encoded = write_runs(&mut grid, &page, &[]).map_err(|error| fault(&error.fault, run.row))?;
        let cells = encoded.runs.first().map_or(0, |report| usize::from(report.cells));
        let line = grid.get(usize::from(run.row)).copied().unwrap_or([SPACE; COLUMNS]);
        let columns = encoded.triplets.into_iter().filter(|triplet| triplet.address < ROW_ADDRESS_BASE).collect();
        Ok((line, columns, cells))
    }

    /// `text` as cells of the page plane from `column` of `row`: codes in
    /// the row, a mark or G2 character over a cell that needs one, and
    /// the set of a run in another language. `before` is the row's cells
    /// before the run.
    ///
    /// # Errors
    /// What the encoder refused, as [`fault`] words it: a `TypeError`
    /// naming a character no set shows, a `RangeError` for a text past
    /// the row or the bound on its characters.
    pub fn plane(&self, row: u8, column: u8, text: &str, language: i32, before: &[PlaneCell]) -> Result<Vec<PlaneCell>, Reply> {
        let language = self.tag(language)?;
        let (line, triplets, count) = self.encode(TextRun { row, column, text, language }, true, before)?;
        let mut cells: Vec<PlaneCell> = line.iter().skip(usize::from(column)).take(count).map(|code| PlaneCell::code(*code)).collect();
        let mut set = None;
        let mut pending = triplets.iter().peekable();
        for (at, cell) in (column..).zip(&mut cells) {
            while let Some(triplet) = pending.next_if(|triplet| triplet.address <= at) {
                if triplet.mode == MODE_ROW_SET {
                    set = Some(triplet.data).filter(|value| *value != self.page_value);
                } else if triplet.address == at {
                    cell.mark = Some((triplet.mode, triplet.data));
                }
            }
            cell.set = set;
        }
        Ok(cells)
    }

    /// `text` as characters of overlay cells: an X/26 triplet each.
    ///
    /// # Errors
    /// As [`Self::plane`].
    pub fn overlay(&self, row: u8, column: u8, text: &str, language: i32) -> Result<Vec<OverlayCharacter>, Reply> {
        let language = Some(self.tag(language)?.unwrap_or(self.page.as_str()));
        let (_, triplets, count) = self.encode(TextRun { row, column, text, language }, false, &[])?;
        let mut characters = Vec::with_capacity(count);
        let mut sets = NO_PAGE_SET;
        for triplet in triplets {
            if triplet.mode == MODE_MODIFIED_DESIGNATION {
                sets = triplet.data;
            } else {
                characters.push(OverlayCharacter { mode: triplet.mode, data: triplet.data, sets });
            }
        }
        if characters.len() == count { Ok(characters) } else { Err(Reply::Error("the encoder gave no triplet for a character of the overlay".to_owned())) }
    }

    /// `nt.teletext.measure`: the cells `text` takes, however many.
    ///
    /// # Errors
    /// As [`Self::plane`], but that a text longer than a row is measured.
    pub fn measure(&self, text: &str, language: i32) -> Result<usize, Reply> {
        let language = self.tag(language)?;
        if text.chars().count() > MAX_TEXT_CHARS {
            return Err(range("MAX_TEXT_CHARS", "a text is at most 1,024 characters"));
        }
        let runs = [TextRun { row: ANY_ROW, column: 0, text, language }];
        let page = PageText { language: Some(self.page.as_str()), designation: self.page_value, level: Level::NeoTeletext, runs: &runs };
        let mut grid = [[SPACE; COLUMNS]; ROWS];
        match write_runs(&mut grid, &page, &[]) {
            Ok(encoded) => Ok(encoded.runs.first().map_or(0, |report| usize::from(report.cells))),
            // The encoder names the last column a run too long for a row
            // would end at, which is one less than its cells.
            Err(error) => match error.fault {
                RunFault::OutOfColumns(last) => Ok(last.saturating_add(1)),
                other => Err(fault(&other, ANY_ROW)),
            },
        }
    }
}

/// The error a script is given for what the encoder refused.
fn fault(fault: &RunFault, row: u8) -> Reply {
    match fault {
        RunFault::Unshown(character) => {
            Reply::Type(format!("the page's character sets cannot show {character:?} (U+{:04X}) in the text's language", u32::from(*character)))
        }
        RunFault::UnknownLanguage(tag) => Reply::Type(format!("no character set writes the language {tag:?}")),
        RunFault::OutOfColumns(last) => range("COLUMNS", &format!("the text would end at column {last} of row {row}, and a row's last column is 39")),
        RunFault::OverBudget(count) => range("LOCAL_TRIPLETS", &format!("the text alone needs {count} triplets")),
        RunFault::SetChanged { row, column } => Reply::Type(format!(
            "the code {ESC} (ESC) at row {row}, column {column} changes the character set before the text; text is put where no ESC comes before it on its row"
        )),
        other => Reply::Type(other.to_string()),
    }
}
