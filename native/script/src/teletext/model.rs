//! What a teletext page's script has drawn, as teletext holds it
//! (script-api.md sections 5.1, 5.2 and 5.4; item 5 of section 13): the
//! Level 1 page plane of rows 1 to 23, whose cells hold codes and whose
//! spacing attributes occupy cells; the overlay, whose cells carry their
//! own look as non-spacing attributes; and the controls. From these the
//! page's local enhancement data is written whole, in the standard's
//! order ([`Model::stream`]): every triplet of it is paid for from the
//! page's [`LOCAL_TRIPLETS`].
//!
//! The overlay lies over the plane without touching a neighbour. At
//! Levels 2.5 and 3.5 a row has one record of attributes, which the Level
//! 1 spacing attributes and the non-spacing attributes both write, and a
//! non-spacing colour holds to the end of the row or the next attribute
//! of its type (EN 300 706 clause 12.3.4 and Annex A.7; the engine's
//! `enhance/row.rs`). So the writer follows that record along each row
//! beside the look the Level 1 codes alone would give, and writes a
//! triplet wherever the record is not what the cell is to show: an
//! overlay cell's own colours where it starts, and the plane's own again
//! at the first cell after it.

use neoteletext_core::charset::{G0Set, G2Set, modified_designation};
use neoteletext_core::consts::{
    ALPHA_BLACK, ALPHA_WHITE, BLACK_BACKGROUND, COLOUR_ATTRIBUTE_MASK, COLOUR_BLACK, COLOUR_WHITE, COLUMNS, CONTROL_JOINED, CONTROL_WIDTH_MASK, FLASH,
    FLASH_MODE_NORMAL, FLASH_MODE_STEADY, FLASH_RATE_SHIFT, FLASH_RATE_SLOW, MODE_ADDITIONAL_FLASH, MODE_BACKGROUND_COLOUR, MODE_CONTROL,
    MODE_FOREGROUND_COLOUR, MODE_MODIFIED_DESIGNATION, MODE_ROW_SET, MODE_SET_ACTIVE_POSITION, MODE_TERMINATION, MOSAIC_BLACK, MOSAIC_WHITE, NEW_BACKGROUND,
    ROW_ADDRESS_BASE, ROW_ADDRESS_LAST, SPACE, STEADY, TERMINATION_LOCAL_DATA_END, TRIPLET_COLOUR_INDEX_MASK,
};
use protocol::script::batch::{FIRST_SCRIPT_ROW, MAX_ROW_CODE, Triplet};

use super::limits::{LOCAL_TRIPLETS, MAX_CONTROLS, PLANE_ROWS};

/// One cell of the page plane.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlaneCell {
    /// Its code, seven bits: a character, a mosaic or a spacing attribute.
    pub code: u8,
    /// The X/26 character the engine's encoder placed over it, as a
    /// triplet's mode and data: a letter with a diacritical mark, or a G2
    /// character (section 5.6).
    pub mark: Option<(u8, u8)>,
    /// The Table 32 value of the set its code is read in when that is not
    /// the page's: a run in another language, written with NeoTeletext's
    /// row set triplet.
    pub set: Option<u8>,
}

impl PlaneCell {
    /// A space of the page's own set.
    pub const BLANK: Self = Self::code(SPACE);

    /// A cell holding `code` alone.
    #[must_use]
    pub const fn code(code: u8) -> Self {
        Self { code, mark: None, set: None }
    }
}

/// The character of an overlay cell: an X/26 triplet's mode and data, and
/// the Table 32 value that designates the sets it is read in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OverlayCharacter {
    /// The triplet's mode: a G0 character, one with a mark, or a G2 one.
    pub mode: u8,
    /// The triplet's data.
    pub data: u8,
    /// The designation its sets are named by.
    pub sets: u8,
}

/// One cell of the overlay; a member that is none is the plane's own.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct OverlayCell {
    /// Its character.
    pub character: Option<OverlayCharacter>,
    /// Its foreground, a Colour Map entry.
    pub ink: Option<u8>,
    /// Its background, a Colour Map entry.
    pub paper: Option<u8>,
    /// Whether it flashes.
    pub flash: Option<bool>,
}

/// One control: a rectangle of cells the page answers itself (the
/// engine's decision E.127).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Control {
    /// Its top row, 1 to 23.
    pub row: u8,
    /// Its first column.
    pub column: u8,
    /// Cells across, 1 to 40.
    pub width: u8,
    /// Rows down: one for its first triplet and one for each joined to it.
    pub height: u8,
    /// The look forced on it: 0 none, 1 hover, 2 pressed.
    pub look: u8,
    /// The scope it belongs to, or 0.
    pub scope: i32,
}

impl Control {
    /// Whether any of its cells is on `row` from `column` to before `end`.
    fn overlaps(&self, row: u8, column: u8, end: u8) -> bool {
        row >= self.row && row < self.row.saturating_add(self.height) && column < self.column.saturating_add(self.width) && self.column < end
    }
}

/// A rectangle of cells, already held to the plane.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rect {
    /// Its top row, 1 to 23.
    pub row: u8,
    /// Its first column.
    pub column: u8,
    /// Rows down.
    pub rows: u8,
    /// Cells across.
    pub columns: u8,
}

/// The colours and flash a cell is drawn with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Look {
    foreground: u8,
    background: u8,
    flash: bool,
}

impl Look {
    /// A spacing attribute that acts from the cell after its own (Table
    /// 26, "set-after"): a colour, or Flash. At Levels 2.5 and 3.5, where
    /// alone the overlay is drawn, Alpha Black and Mosaic Black are black
    /// (the engine's REQ-L25-022).
    const fn set_after(&mut self, code: u8) {
        match code {
            ALPHA_BLACK..=ALPHA_WHITE | MOSAIC_BLACK..=MOSAIC_WHITE => self.foreground = code & COLOUR_ATTRIBUTE_MASK,
            FLASH => self.flash = true,
            _ => {}
        }
    }

    /// A spacing attribute that acts in its own cell ("set-at"): Steady
    /// and the two background codes. `colour` is the colour the Level 1
    /// codes last selected, which New Background takes, and `black` what
    /// Black Background gives.
    const fn set_at(&mut self, code: u8, colour: u8, black: u8) {
        match code {
            STEADY => self.flash = false,
            BLACK_BACKGROUND => self.background = black,
            NEW_BACKGROUND => self.background = colour,
            _ => {}
        }
    }
}

/// Whether `code` is a colour attribute, alphanumeric or mosaic.
const fn is_colour(code: u8) -> bool {
    matches!(code, ALPHA_BLACK..=ALPHA_WHITE | MOSAIC_BLACK..=MOSAIC_WHITE)
}

/// What the Level 1 codes of a row give at each of its cells, with no
/// enhancement: the foreground in force, for a `put` that must restore it.
#[must_use]
pub fn foreground_at(codes: &[PlaneCell; COLUMNS], column: usize) -> u8 {
    let mut look = Look { foreground: COLOUR_WHITE, background: COLOUR_BLACK, flash: false };
    for cell in codes.iter().take(column) {
        look.set_after(cell.code);
    }
    look.foreground
}

/// The page's settings (EN 300 706 clause 9.4.2, X/28/0 format 1).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PageSettings {
    /// The default screen colour, a Colour Map entry.
    pub screen: u8,
    /// The default row colour, a Colour Map entry.
    pub row_colour: u8,
    /// Black background colour substitution: a black background is drawn
    /// in the row colour.
    pub substitution: bool,
}

/// Everything the page's local enhancement data is written from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Model {
    /// The page plane, rows 1 to 23.
    pub plane: [[PlaneCell; COLUMNS]; PLANE_ROWS],
    /// The overlay, over the same cells.
    pub overlay: [[OverlayCell; COLUMNS]; PLANE_ROWS],
    /// The controls, in the order they were made.
    pub controls: Vec<Control>,
    /// The page's settings.
    pub settings: PageSettings,
    /// The colours the sixel bitmap's frame sets under the overlay
    /// (section 5.7; work package T3).
    pub bitmap: super::bitmap::Colours,
}

/// The place of `row` among the plane's rows.
fn index(row: u8) -> usize {
    usize::from(row.saturating_sub(FIRST_SCRIPT_ROW))
}

/// The row at `index` of the plane.
fn row_at(index: usize) -> u8 {
    u8::try_from(index).unwrap_or(u8::MAX).saturating_add(FIRST_SCRIPT_ROW)
}

/// A column triplet.
const fn column_triplet(column: u8, mode: u8, data: u8) -> Triplet {
    Triplet { address: column, mode, data }
}

impl Model {
    /// A blank plane with no overlay and no control.
    #[must_use]
    pub fn new() -> Box<Self> {
        Box::new(Self {
            plane: [[PlaneCell::BLANK; COLUMNS]; PLANE_ROWS],
            overlay: [[OverlayCell::default(); COLUMNS]; PLANE_ROWS],
            controls: Vec::with_capacity(MAX_CONTROLS),
            settings: PageSettings::default(),
            bitmap: super::bitmap::Colours::default(),
        })
    }

    /// The cells of `row`.
    #[must_use]
    pub fn row(&self, row: u8) -> Option<&[PlaneCell; COLUMNS]> {
        self.plane.get(index(row))
    }

    /// Writes `cells` on `row` from `column`; the caller has held them to
    /// the row.
    pub fn write(&mut self, row: u8, column: u8, cells: impl Iterator<Item = PlaneCell>) {
        if let Some(line) = self.plane.get_mut(index(row)) {
            for (slot, cell) in line.iter_mut().skip(usize::from(column)).zip(cells) {
                *slot = cell;
            }
        }
    }

    /// Fills the rows `first` to `last` with spaces.
    pub fn clear_rows(&mut self, first: u8, last: u8) {
        for line in self.plane.iter_mut().skip(index(first)).take(usize::from(last.saturating_sub(first)).saturating_add(1)) {
            *line = [PlaneCell::BLANK; COLUMNS];
        }
    }

    /// Moves the rows of the band `first` to `last` down by `rows`, or up
    /// when it is negative; the rows vacated are spaces, and rows moved
    /// out of the band are gone.
    pub fn scroll(&mut self, first: u8, last: u8, rows: i32) {
        let (top, bottom) = (index(first), index(last));
        let Some(band) = self.plane.get_mut(top..=bottom) else { return };
        let distance = usize::try_from(rows.unsigned_abs()).unwrap_or(usize::MAX).min(band.len());
        if rows < 0 {
            band.rotate_left(distance);
            let kept = band.len().saturating_sub(distance);
            for line in band.iter_mut().skip(kept) {
                *line = [PlaneCell::BLANK; COLUMNS];
            }
        } else {
            band.rotate_right(distance);
            for line in band.iter_mut().take(distance) {
                *line = [PlaneCell::BLANK; COLUMNS];
            }
        }
    }

    /// Replaces the whole plane with `codes`, row by row; the caller has
    /// held their number and each code.
    pub fn blit(&mut self, codes: &[u8]) {
        for (line, codes) in self.plane.iter_mut().zip(codes.chunks(COLUMNS)) {
            for (slot, code) in line.iter_mut().zip(codes) {
                *slot = PlaneCell::code(*code & MAX_ROW_CODE);
            }
        }
    }

    /// The overlay cells of `rect`, each given to `change`.
    pub fn overlay_each(&mut self, rect: Rect, mut change: impl FnMut(usize, &mut OverlayCell)) {
        for line in self.overlay.iter_mut().skip(index(rect.row)).take(usize::from(rect.rows)) {
            for (offset, cell) in line.iter_mut().skip(usize::from(rect.column)).take(usize::from(rect.columns)).enumerate() {
                change(offset, cell);
            }
        }
    }

    /// Removes the whole overlay.
    pub fn overlay_clear(&mut self) {
        self.overlay = [[OverlayCell::default(); COLUMNS]; PLANE_ROWS];
    }

    /// The control whose first cell is at `row`, `column`, by its place.
    #[must_use]
    pub fn control_at(&self, row: u8, column: u8) -> Option<usize> {
        self.controls.iter().position(|control| control.row == row && control.column == column)
    }

    /// Whether a control has a cell on `row` from `column` to before `end`.
    #[must_use]
    pub fn control_over(&self, row: u8, column: u8, end: u8) -> bool {
        self.controls.iter().any(|control| control.overlaps(row, column, end))
    }

    /// The control that ends on the row above `row` at the same column and
    /// width: the one a joined run extends (decision E.127).
    #[must_use]
    pub fn control_above(&self, row: u8, column: u8, width: u8) -> Option<usize> {
        self.controls.iter().position(|control| control.row.saturating_add(control.height) == row && control.column == column && control.width == width)
    }

    /// The controls in the order the engine numbers them from 1: the order
    /// their first triplets come in the local enhancement data, which is
    /// by row and then by column.
    #[must_use]
    pub fn controls_in_order(&self) -> Vec<Control> {
        let mut ordered = self.controls.clone();
        ordered.sort_by_key(|control| (control.row, control.column));
        ordered
    }

    /// The page's local enhancement data, whole, in the order EN 300 706
    /// Annex A.4 requires: row by row from the top, each row's address
    /// triplet and then its column triplets by column, and the
    /// Termination Marker; nothing at all when no row needs a triplet.
    /// `page_value` is the Table 32 value of the page's own set. Writing
    /// stops one past [`LOCAL_TRIPLETS`], which is enough to know the
    /// page cannot hold it.
    #[must_use]
    pub fn stream(&self, page_value: u8) -> Vec<Triplet> {
        let mut out: Vec<Triplet> = Vec::new();
        let mut row_triplets: Vec<Triplet> = Vec::with_capacity(COLUMNS);
        for (at, (plane, overlay)) in self.plane.iter().zip(&self.overlay).enumerate() {
            let row = row_at(at);
            row_triplets.clear();
            self.row_stream(row, plane, overlay, page_value, &mut row_triplets);
            if row_triplets.is_empty() {
                continue;
            }
            out.push(Triplet { address: ROW_ADDRESS_BASE.saturating_add(row), mode: MODE_SET_ACTIVE_POSITION, data: 0 });
            out.append(&mut row_triplets);
            if out.len() > LOCAL_TRIPLETS {
                return out;
            }
        }
        if !out.is_empty() {
            out.push(Triplet { address: ROW_ADDRESS_LAST, mode: MODE_TERMINATION, data: TERMINATION_LOCAL_DATA_END });
        }
        out
    }

    /// The column triplets of one row, by column: at a column, a change
    /// of set first, then the colours and flash, then a control, then the
    /// character.
    fn row_stream(&self, row: u8, plane: &[PlaneCell; COLUMNS], overlay: &[OverlayCell; COLUMNS], page_value: u8, out: &mut Vec<Triplet>) {
        // What Black Background and the start of a row give (the engine's
        // REQ-L25-022): black, or the row colour under substitution.
        let black = if self.settings.substitution { self.settings.row_colour } else { COLOUR_BLACK };
        // The look the Level 1 codes alone give, and the record the
        // receiver holds, which the triplets written here also change.
        let mut level_1 = Look { foreground: COLOUR_WHITE, background: black, flash: false };
        let mut record = level_1;
        // The colour the Level 1 codes last selected, for New Background.
        let mut colour = COLOUR_WHITE;
        // The set the row's codes are read in, and the sets X/26
        // characters are read in.
        let mut row_set = page_value;
        let mut sets: Option<(G0Set, G2Set)> = modified_designation(page_value);
        let mut previous: Option<u8> = None;
        for ((column, cell), over) in (0_u8..).zip(plane).zip(overlay) {
            if let Some(before) = previous {
                level_1.set_after(before);
                record.set_after(before);
                if is_colour(before) {
                    colour = before & COLOUR_ATTRIBUTE_MASK;
                }
            }
            level_1.set_at(cell.code, colour, black);
            record.set_at(cell.code, colour, black);
            previous = Some(cell.code);
            // The sixel bitmap's own colour triplets (section 5.7): a
            // non-spacing colour, which holds along the row as the Level
            // 1 look does, and which the overlay lies over.
            let (ink, paper) = self.bitmap.at(row, column);
            level_1.foreground = ink.unwrap_or(level_1.foreground);
            level_1.background = paper.unwrap_or(level_1.background);
            // A code that shows a character is read in its run's set, or
            // in the page's when it belongs to no run (the engine's
            // REQ-2180): a row set triplet where the set in force is
            // another.
            if cell.set.is_some() || cell.code > SPACE {
                let wanted = cell.set.unwrap_or(page_value);
                if wanted != row_set {
                    out.push(column_triplet(column, MODE_ROW_SET, wanted));
                    row_set = wanted;
                    sets = modified_designation(wanted);
                }
            }
            // The cell's X/26 character: the overlay's, or else the mark
            // the encoder placed on the plane's.
            let character = match (over.character, cell.mark) {
                (Some(character), _) => Some((character.mode, character.data, character.sets)),
                (None, Some((mode, data))) => Some((mode, data, cell.set.unwrap_or(page_value))),
                (None, None) => None,
            };
            if let Some((_, _, value)) = character {
                let wanted = modified_designation(value);
                if wanted.is_some() && wanted != sets {
                    out.push(column_triplet(column, MODE_MODIFIED_DESIGNATION, value));
                    sets = wanted;
                }
            }
            let wanted = Look {
                foreground: over.ink.unwrap_or(level_1.foreground),
                background: over.paper.unwrap_or(level_1.background),
                flash: over.flash.unwrap_or(level_1.flash),
            };
            if record.foreground != wanted.foreground {
                out.push(column_triplet(column, MODE_FOREGROUND_COLOUR, wanted.foreground & TRIPLET_COLOUR_INDEX_MASK));
            }
            if record.background != wanted.background {
                out.push(column_triplet(column, MODE_BACKGROUND_COLOUR, wanted.background & TRIPLET_COLOUR_INDEX_MASK));
            }
            if record.flash != wanted.flash {
                // Table 29, mode 00111: steady, or normal flash at the
                // slow rate, which is the Level 1 flash's.
                let data = if wanted.flash { FLASH_MODE_NORMAL | (FLASH_RATE_SLOW << FLASH_RATE_SHIFT) } else { FLASH_MODE_STEADY };
                out.push(column_triplet(column, MODE_ADDITIONAL_FLASH, data));
            }
            record = wanted;
            for control in &self.controls {
                if control.column == column && row >= control.row && row < control.row.saturating_add(control.height) {
                    let joined = if row > control.row { CONTROL_JOINED } else { 0 };
                    out.push(column_triplet(column, MODE_CONTROL, (control.width & CONTROL_WIDTH_MASK) | joined));
                }
            }
            if let Some((mode, data, _)) = character {
                out.push(column_triplet(column, mode, data));
            }
        }
    }
}
