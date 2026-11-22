//! The teletext API's host (work package T1 of script-api.md; sections
//! 5.1, 5.2, 5.4, 5.5 and 5.6, operations 100 to 119 of section 10.2;
//! DEC-7247 item 2). Teletext is programmed as what it is, not behind a
//! drawing surface:
//!
//! - **the page plane** ([`model`]): rows 1 to 23 of forty cells, each a
//!   seven-bit code. A spacing attribute occupies a cell, so `page.put`
//!   places the attributes its style needs in the cells before its text,
//!   as a Level 1 page is authored (EN 300 706 clause 12.2, Table 26),
//!   and refuses by `RangeError TELETEXT_ATTRIBUTE_CELL` when one of
//!   those cells is not free;
//! - **the overlay**: cells that carry their own look as non-spacing
//!   attributes, the page's local enhancement data (packets X/26, clause
//!   12.3), every triplet of it paid for from [`LOCAL_TRIPLETS`];
//! - **controls**: cells the page answers itself, by NeoTeletext's
//!   control triplet (the engine's decision E.127);
//! - **the Colour Map** ([`colours`]): entries 16 to 31, redefined by
//!   X/28/0 format 1;
//! - **text** ([`text`]): through the engine's own encoder, for the
//!   page's language or a run's own;
//! - **the sixel bitmap** ([`bitmap`], work package T3): two buffers of
//!   sixels beside these, which `present()` has the engine turn into the
//!   codes of the plane's rows it owns and into triplets of the same
//!   local enhancement data.
//!
//! The host holds the page as the script drew it and, for each step,
//! stages only what changed as the batch's sections `TELETEXT_ROWS`,
//! `TELETEXT_ENHANCEMENT`, `TELETEXT_SETTINGS`, `ANIMATE` and `LABELS`
//! ([`Teletext::fill`]). It writes no packet: the decoder does (section
//! 11.2). Everything here is fixed in size, so a script cannot make the
//! host hold more for teletext than a page (section 10.1 rule 7).
//!
//! Every operation validates all of its arguments and the page's triplet
//! budget before it changes anything (rule 3): a change is made to a copy
//! of the page, the copy's enhancement data is written and counted, and
//! only a copy within the budget replaces the page.

pub mod bitmap;
pub mod colours;
pub mod limits;
pub mod model;
pub mod text;

use std::fmt::Write as _;

use neoteletext_core::consts::{
    ALPHA_BLACK, BLACK_BACKGROUND, COLOUR_BLACK, COLUMNS, CONCEAL, CONTIGUOUS_MOSAICS, DOUBLE_HEIGHT, END_BOX, FLASH, HOLD_MOSAICS, MOSAIC_BLACK,
    NEW_BACKGROUND, NORMAL_SIZE, RELEASE_MOSAICS, SEPARATED_MOSAICS, SPACE, START_BOX, STEADY,
};
use neoteletext_rules::animate::{Operation, OperationKind, code};
use neoteletext_rules::table::Target;
use protocol::script::batch::{Label, MAX_ROW_CODE, TeletextRow, TeletextSettings, Triplet};
use protocol::script::teletext::{control_cell, control_handle};
use protocol::script::{Batch, ContextId, ContextOpen, Renderer};

use self::colours::ColourMap;
use self::limits::{
    BLIT_CODES, FIRST_ROW, LAST_COLOUR8, LAST_COLUMN, LAST_CONTROL_LOOK, LAST_DOUBLE_HEIGHT_ROW, LAST_ENTRY, LAST_ROW, LOCAL_TRIPLETS, MAX_CONTROLS,
    PLANE_ROWS, TELETEXT_DEFINABLE_COLOURS,
};
use self::model::{Control, Model, OverlayCell, PageSettings, PlaneCell, Rect, foreground_at};
use self::text::Languages;
use crate::host::{Payload, Reply};
use crate::limits::MAX_LABEL_BYTES;
use crate::ops::Op;

/// The script-side half of the teletext API, which follows the shared
/// core's in the prelude.
pub const PRELUDE: &str = include_str!("teletext.js");

/// How a style member that is on or off crosses the host call, two bits
/// each of an operation's `flags` (this session's packing; section 10.2
/// says only "flags"): not given, off, on.
const NOT_GIVEN: i32 = 0;
const OFF: i32 = 1;
const ON: i32 = 2;
/// `mosaic` has a third value beside off (`false`) and on
/// (`"contiguous"`): `"separated"`.
const SEPARATED: i32 = 3;
/// The mask of one style member's two bits.
const MEMBER_MASK: i32 = 3;
/// Where each style member's bits lie in `flags`.
const FLASH_SHIFT: u32 = 0;
const CONCEAL_SHIFT: u32 = 2;
const BOX_SHIFT: u32 = 4;
const HOLD_SHIFT: u32 = 6;
const MOSAIC_SHIFT: u32 = 8;
/// One past the greatest `flags` of `page.put`, and of `overlay.put`,
/// which has `flash` alone.
const PUT_FLAGS_END: i32 = 1 << (MOSAIC_SHIFT + 2);
const OVERLAY_FLAGS_END: i32 = 1 << (FLASH_SHIFT + 2);
/// An argument that stands for "not given" or "leave as it is".
const NONE: i32 = -1;

/// The numbers the prelude's teletext part shares with this file, written
/// in front of the prelude as the table `TT` (section 10.1: defined once,
/// in Rust).
#[must_use]
pub fn prelude_numbers() -> [(&'static str, i64); 14] {
    [
        ("NOT_GIVEN", i64::from(NOT_GIVEN)),
        ("OFF", i64::from(OFF)),
        ("ON", i64::from(ON)),
        ("SEPARATED", i64::from(SEPARATED)),
        ("FLASH_SHIFT", i64::from(FLASH_SHIFT)),
        ("CONCEAL_SHIFT", i64::from(CONCEAL_SHIFT)),
        ("BOX_SHIFT", i64::from(BOX_SHIFT)),
        ("HOLD_SHIFT", i64::from(HOLD_SHIFT)),
        ("MOSAIC_SHIFT", i64::from(MOSAIC_SHIFT)),
        ("NONE", i64::from(NONE)),
        ("LOOK", i64::from(code::LOOK)),
        ("FIRST_ROW", i64::from(FIRST_ROW)),
        ("LAST_ROW", i64::from(LAST_ROW)),
        ("LAST_COLUMN", i64::from(LAST_COLUMN)),
    ]
}

/// A `RangeError` naming `bound`.
fn range(bound: &str, detail: &str) -> Reply {
    Reply::Range(format!("{bound}: {detail}"))
}

/// A `TypeError`.
fn type_error(detail: &str) -> Reply {
    Reply::Type(detail.to_owned())
}

/// Exactly `N` whole numbers, or a `TypeError` saying so.
fn exactly<const N: usize>(op: Op, ints: &[i32]) -> Result<[i32; N], Reply> {
    <[i32; N]>::try_from(ints).map_err(|_| Reply::Type(format!("operation {op:?} takes {N} whole numbers, not {}", ints.len())))
}

/// The payload as a string, or a `TypeError`.
fn text_of(op: Op, payload: Payload<'_>) -> Result<&str, Reply> {
    match payload {
        Payload::Text(bytes) => core::str::from_utf8(bytes).map_err(|_| Reply::Type(format!("operation {op:?}'s string is not well-formed Unicode"))),
        _ => Err(Reply::Type(format!("operation {op:?} takes a string"))),
    }
}

/// The payload as bytes, or a `TypeError`.
fn bytes_of(op: Op, payload: Payload<'_>) -> Result<&[u8], Reply> {
    match payload {
        Payload::Bytes(bytes) => Ok(bytes),
        _ => Err(Reply::Type(format!("operation {op:?} takes a Uint8Array"))),
    }
}

/// No payload, or a `TypeError`.
fn no_payload(op: Op, payload: Payload<'_>) -> Result<(), Reply> {
    if payload == Payload::None { Ok(()) } else { Err(Reply::Type(format!("operation {op:?} takes no payload"))) }
}

/// `value` from `low` to `high` as a byte, or a `RangeError` naming
/// `bound`.
fn within(value: i32, low: i32, high: i32, bound: &str, what: &str) -> Result<u8, Reply> {
    u8::try_from(value).ok().filter(|_| (low..=high).contains(&value)).ok_or_else(|| Reply::Range(format!("{bound}: {what} is {low} to {high}, not {value}")))
}

/// A row of the plane, 1 to 23.
fn row_of(row: i32) -> Result<u8, Reply> {
    within(row, FIRST_ROW, LAST_ROW, "TELETEXT_ROWS", "a row")
}

/// A column, 0 to 39.
fn column_of(column: i32) -> Result<u8, Reply> {
    within(column, 0, LAST_COLUMN, "COLUMNS", "a column")
}

/// A colour that may be left out: [`NONE`], or 0 to `last`.
fn optional(value: i32, last: i32, bound: &str, what: &str) -> Result<Option<u8>, Reply> {
    if value == NONE { Ok(None) } else { within(value, 0, last, bound, what).map(Some) }
}

/// The two bits of the style member at `shift` of `flags`.
const fn member(flags: i32, shift: u32) -> i32 {
    (flags >> shift) & MEMBER_MASK
}

/// A style member that is given as on or off, or not given.
fn on_off(flags: i32, shift: u32, what: &str) -> Result<Option<bool>, Reply> {
    match member(flags, shift) {
        NOT_GIVEN => Ok(None),
        OFF => Ok(Some(false)),
        ON => Ok(Some(true)),
        _ => Err(Reply::Type(format!("{what} is true or false"))),
    }
}

/// A rectangle of cells inside the plane, or the `RangeError`.
fn rect_of(row: i32, column: i32, rows: i32, columns: i32) -> Result<Rect, Reply> {
    let (top, left) = (row_of(row)?, column_of(column)?);
    let rows = within(rows, 1, LAST_ROW.saturating_sub(row).saturating_add(1), "TELETEXT_ROWS", "the rows of a rectangle from this row")?;
    let columns = within(columns, 1, LAST_COLUMN.saturating_sub(column).saturating_add(1), "COLUMNS", "the columns of a rectangle from this column")?;
    Ok(Rect { row: top, column: left, rows, columns })
}

/// The style member `mosaic`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mosaic {
    /// `false`: alphanumerics.
    Off,
    /// `"contiguous"`.
    Contiguous,
    /// `"separated"`.
    Separated,
}

/// What `page.put`'s style asks for (section 5.1).
#[derive(Clone, Copy, Debug)]
struct Style {
    ink: Option<u8>,
    paper: Option<u8>,
    double_height: Option<bool>,
    flash: Option<bool>,
    conceal: Option<bool>,
    boxed: Option<bool>,
    mosaic: Option<Mosaic>,
    hold: Option<bool>,
}

impl Style {
    /// The spacing attributes the style needs, in the order they are
    /// placed before the text (EN 300 706 clause 12.2, Table 26). One
    /// attribute for each member given, whatever the row already holds,
    /// so that an author knows what a style costs:
    ///
    /// - `paper`: Black Background for black; otherwise the colour as an
    ///   alphanumeric colour and New Background, which takes the
    ///   foreground in force as the background ("set-at"). The colour
    ///   code changed the foreground, so the ink follows;
    /// - `box`: Start Box or End Box, twice, as clause 12.2 has both sent;
    /// - `height`: Double Height ("set-after") or Normal Size ("set-at");
    /// - `flash`: Flash ("set-after") or Steady ("set-at");
    /// - `mosaic`: Contiguous or Separated Mosaics ("set-at"), and the
    ///   ink as a mosaic colour; `false`, the ink as an alphanumeric
    ///   colour;
    /// - `hold`: Hold Mosaics ("set-at") or Release Mosaics ("set-after");
    /// - `ink`: the alphanumeric or mosaic colour ("set-after"). When
    ///   `paper` or `mosaic` needs it and no ink is given it is
    ///   `in_force`, the foreground the row has where the attributes
    ///   begin;
    /// - `conceal`: Conceal ("set-at"), last, since a colour ends it.
    ///
    /// A "set-after" attribute acts from the next cell and every
    /// attribute lies before the text, so all are in force at the text's
    /// first character.
    fn attributes(&self, in_force: u8) -> Vec<u8> {
        let mut codes = Vec::new();
        let mut ink_needed = self.ink.is_some();
        match self.paper {
            Some(COLOUR_BLACK) => codes.push(BLACK_BACKGROUND),
            Some(colour) => {
                codes.extend([ALPHA_BLACK | colour, NEW_BACKGROUND]);
                ink_needed = true;
            }
            None => {}
        }
        match self.boxed {
            Some(true) => codes.extend([START_BOX, START_BOX]),
            Some(false) => codes.extend([END_BOX, END_BOX]),
            None => {}
        }
        match self.double_height {
            Some(true) => codes.push(DOUBLE_HEIGHT),
            Some(false) => codes.push(NORMAL_SIZE),
            None => {}
        }
        match self.flash {
            Some(true) => codes.push(FLASH),
            Some(false) => codes.push(STEADY),
            None => {}
        }
        match self.mosaic {
            Some(Mosaic::Separated) => codes.push(SEPARATED_MOSAICS),
            Some(Mosaic::Contiguous) => codes.push(CONTIGUOUS_MOSAICS),
            Some(Mosaic::Off) | None => {}
        }
        ink_needed |= self.mosaic.is_some();
        match self.hold {
            Some(true) => codes.push(HOLD_MOSAICS),
            Some(false) => codes.push(RELEASE_MOSAICS),
            None => {}
        }
        if ink_needed {
            let base = if matches!(self.mosaic, Some(Mosaic::Contiguous | Mosaic::Separated)) { MOSAIC_BLACK } else { ALPHA_BLACK };
            codes.push(base | self.ink.unwrap_or(in_force));
        }
        if self.conceal == Some(true) {
            codes.push(CONCEAL);
        }
        codes
    }
}

/// What the host holds for the teletext page a script draws on.
pub struct Teletext {
    /// The page as the script drew it.
    model: Box<Model>,
    /// The triplets its local enhancement data takes.
    used: usize,
    colours: ColourMap,
    languages: Languages,
    /// The receiver's presentation level: 10, 15, 25 or 35.
    level: u8,
    /// What the decoder was last told: the rows' codes, the enhancement
    /// data, the settings, and the look forced on each control by the
    /// number the engine gives it.
    rows_told: Box<[[u8; COLUMNS]; PLANE_ROWS]>,
    stream_told: Vec<Triplet>,
    settings_told: TeletextSettings,
    looks_told: [u8; MAX_CONTROLS],
    /// Labels not yet told, one at most for a control.
    labels: Vec<Label>,
    /// The sixel bitmap's two buffers (section 5.7).
    bitmap: Box<bitmap::Bitmap>,
}

impl Teletext {
    /// A blank page for the context `open` opens: the decoder shows the
    /// script a blank plane, no enhancement and the default settings.
    fn new(open: &ContextOpen) -> Box<Self> {
        Box::new(Self {
            model: Model::new(),
            used: 0,
            colours: ColourMap::default(),
            languages: Languages::new(&open.language),
            level: open.level.code(),
            rows_told: Box::new([[SPACE; COLUMNS]; PLANE_ROWS]),
            stream_told: Vec::new(),
            settings_told: TeletextSettings::default(),
            looks_told: [0; MAX_CONTROLS],
            labels: Vec::new(),
            bitmap: bitmap::Bitmap::new(),
        })
    }

    /// The bytes held: fixed, but for the labels waiting, which are at
    /// most one of [`MAX_LABEL_BYTES`] for each control.
    fn state_bytes(&self) -> usize {
        let fixed = size_of::<Self>()
            .saturating_add(size_of::<Model>())
            .saturating_add(MAX_CONTROLS.saturating_mul(size_of::<Control>()))
            .saturating_add(size_of::<[[u8; COLUMNS]; PLANE_ROWS]>())
            .saturating_add(LOCAL_TRIPLETS.saturating_mul(size_of::<Triplet>()).saturating_mul(2))
            .saturating_add(self.languages.state_bytes())
            .saturating_add(size_of::<bitmap::Bitmap>());
        fixed.saturating_add(self.batch_bytes())
    }

    /// The bytes of the labels waiting to leave.
    fn batch_bytes(&self) -> usize {
        self.labels.iter().fold(0, |sum, label| sum.saturating_add(size_of::<Label>()).saturating_add(label.label.len()))
    }

    /// Makes a change to a copy of the page and keeps the copy only when
    /// its local enhancement data fits the page's triplets (section 5.2):
    /// otherwise `RangeError LOCAL_TRIPLETS` and nothing has changed.
    fn change<T>(&mut self, edit: impl FnOnce(&mut Model) -> Result<T, Reply>) -> Result<T, Reply> {
        let mut next = self.model.clone();
        let value = edit(&mut next)?;
        let used = next.stream(self.languages.page_value()).len();
        if used > LOCAL_TRIPLETS {
            return Err(range(
                "LOCAL_TRIPLETS",
                &format!("the page's local enhancement data would pass its {LOCAL_TRIPLETS} triplets; it holds {} now", self.used),
            ));
        }
        self.model = next;
        self.used = used;
        Ok(value)
    }

    /// `page.put` (section 5.1).
    fn put(&mut self, op: Op, ints: &[i32], payload: Payload<'_>) -> Result<Reply, Reply> {
        let [row, column, ink, paper, height, flags, language] = exactly(op, ints)?;
        let text = text_of(op, payload)?;
        let (line, at) = (row_of(row)?, column_of(column)?);
        if !(0..PUT_FLAGS_END).contains(&flags) {
            return Err(type_error("the style holds a member this host does not know"));
        }
        let style = Style {
            ink: optional(ink, LAST_COLOUR8, "COLOURS", "a Level 1 colour")?,
            paper: optional(paper, LAST_COLOUR8, "COLOURS", "a Level 1 colour")?,
            double_height: match height {
                0 => None,
                1 => Some(false),
                2 => Some(true),
                _ => return Err(type_error("height is 1 or 2")),
            },
            flash: on_off(flags, FLASH_SHIFT, "flash")?,
            conceal: on_off(flags, CONCEAL_SHIFT, "conceal")?,
            boxed: on_off(flags, BOX_SHIFT, "box")?,
            mosaic: match member(flags, MOSAIC_SHIFT) {
                NOT_GIVEN => None,
                OFF => Some(Mosaic::Off),
                ON => Some(Mosaic::Contiguous),
                _ => Some(Mosaic::Separated),
            },
            hold: on_off(flags, HOLD_SHIFT, "hold")?,
        };
        if style.double_height == Some(true) && row > LAST_DOUBLE_HEIGHT_ROW {
            return Err(range(
                "DOUBLE_HEIGHT_LAST_ROW",
                &format!("double height takes the row below, so it starts on rows {FIRST_ROW} to {LAST_DOUBLE_HEIGHT_ROW}, not {row}"),
            ));
        }
        let cells = self.model.row(line).ok_or_else(|| type_error("no such row"))?;
        // The attributes' number does not depend on the row; the ink that
        // restores the foreground does.
        let needed = style.attributes(COLOUR_BLACK).len();
        let Some(start) = usize::from(at).checked_sub(needed) else {
            return Err(range(
                "TELETEXT_ATTRIBUTE_CELL",
                &format!("the style needs {needed} attribute cells before row {row}, column {column}, and the row has {column} cells there"),
            ));
        };
        let attributes = style.attributes(foreground_at(cells, start));
        // A cell an attribute goes in is free: it holds a space, or the
        // very attribute already, as it does when the same text is put
        // again.
        for ((place, cell), attribute) in cells.iter().enumerate().skip(start).zip(&attributes) {
            if cell.code != SPACE && (cell.code != *attribute || cell.mark.is_some()) {
                return Err(range(
                    "TELETEXT_ATTRIBUTE_CELL",
                    &format!("row {row}, column {place} holds the code {}, and the style needs that cell for a spacing attribute", cell.code),
                ));
            }
        }
        let before = cells.get(..usize::from(at)).unwrap_or_default();
        let written = self.languages.plane(line, at, text, language, before)?;
        let count = attributes.len().saturating_add(written.len());
        let first = u8::try_from(start).unwrap_or(at);
        self.change(|model| {
            model.write(line, first, attributes.iter().map(|code| PlaneCell::code(*code)).chain(written));
            Ok(())
        })?;
        Ok(Reply::Int(i32::try_from(count).unwrap_or(i32::MAX)))
    }

    /// `page.codes`, `page.blit`, `page.at`, `page.clear`, `page.scroll`
    /// and `page.settings` (section 5.1).
    fn plane(&mut self, op: Op, ints: &[i32], payload: Payload<'_>) -> Result<Reply, Reply> {
        let seven_bits = |codes: &[u8]| match codes.iter().find(|code| **code > MAX_ROW_CODE) {
            Some(code) => Err(range("MAX_ROW_CODE", &format!("a code is seven bits, 0 to {MAX_ROW_CODE}, not {code}"))),
            None => Ok(()),
        };
        match op {
            Op::TT_CODES => {
                let [row, column] = exactly(op, ints)?;
                let codes = bytes_of(op, payload)?;
                let (line, at) = (row_of(row)?, column_of(column)?);
                if codes.len() > COLUMNS.saturating_sub(usize::from(at)) {
                    return Err(range("COLUMNS", &format!("{} codes from column {column} would pass the row's last column, 39", codes.len())));
                }
                seven_bits(codes)?;
                self.change(|model| {
                    model.write(line, at, codes.iter().map(|code| PlaneCell::code(*code)));
                    Ok(())
                })?;
            }
            Op::TT_BLIT => {
                let [] = exactly(op, ints)?;
                let codes = bytes_of(op, payload)?;
                if codes.len() != BLIT_CODES {
                    return Err(range("BLIT_CODES", &format!("the plane is exactly {BLIT_CODES} codes, {PLANE_ROWS} rows of {COLUMNS}, not {}", codes.len())));
                }
                seven_bits(codes)?;
                self.change(|model| {
                    model.blit(codes);
                    Ok(())
                })?;
            }
            Op::TT_AT => {
                let [row, column] = exactly(op, ints)?;
                no_payload(op, payload)?;
                let (line, at) = (row_of(row)?, column_of(column)?);
                let code = self.model.row(line).and_then(|cells| cells.get(usize::from(at))).map_or(SPACE, |cell| cell.code);
                return Ok(Reply::Int(i32::from(code)));
            }
            Op::TT_CLEAR | Op::TT_SCROLL => {
                let (first, last, rows) = if op == Op::TT_CLEAR {
                    let [first, last] = exactly(op, ints)?;
                    (first, last, 0)
                } else {
                    let [first, last, rows] = exactly(op, ints)?;
                    (first, last, rows)
                };
                no_payload(op, payload)?;
                let (top, bottom) = (row_of(first)?, row_of(last)?);
                if top > bottom {
                    return Err(range("TELETEXT_ROWS", &format!("the first row, {first}, is after the last, {last}")));
                }
                self.change(|model| {
                    if op == Op::TT_CLEAR {
                        model.clear_rows(top, bottom);
                    } else {
                        model.scroll(top, bottom, rows);
                    }
                    Ok(())
                })?;
            }
            _ => {
                let [screen, row_colour, substitution] = exactly(op, ints)?;
                no_payload(op, payload)?;
                let screen = optional(screen, LAST_ENTRY, "COLOUR_MAP_ENTRIES", "a Colour Map entry")?;
                let row_colour = optional(row_colour, LAST_ENTRY, "COLOUR_MAP_ENTRIES", "a Colour Map entry")?;
                let substitution = match substitution {
                    NONE => None,
                    0 => Some(false),
                    1 => Some(true),
                    _ => return Err(type_error("substitution is true or false")),
                };
                self.change(|model| {
                    let held = model.settings;
                    model.settings = PageSettings {
                        screen: screen.unwrap_or(held.screen),
                        row_colour: row_colour.unwrap_or(held.row_colour),
                        substitution: substitution.unwrap_or(held.substitution),
                    };
                    Ok(())
                })?;
            }
        }
        Ok(Reply::Undefined)
    }

    /// `overlay.put`, `overlay.fill` and `overlay.clear` (section 5.2).
    fn overlay(&mut self, op: Op, ints: &[i32], payload: Payload<'_>) -> Result<Reply, Reply> {
        let entry = |value: i32| optional(value, LAST_ENTRY, "COLOUR_MAP_ENTRIES", "a Colour Map entry");
        match op {
            Op::TT_OVERLAY_PUT => {
                let [row, column, ink, paper, flags, language] = exactly(op, ints)?;
                let text = text_of(op, payload)?;
                let (line, at) = (row_of(row)?, column_of(column)?);
                let (ink, paper) = (entry(ink)?, entry(paper)?);
                if !(0..OVERLAY_FLAGS_END).contains(&flags) {
                    return Err(type_error("the look holds a member this host does not know"));
                }
                let flash = on_off(flags, FLASH_SHIFT, "flash")?;
                let characters = self.languages.overlay(line, at, text, language)?;
                let columns = u8::try_from(characters.len()).unwrap_or(u8::MAX);
                let rect = Rect { row: line, column: at, rows: 1, columns };
                self.change(|model| {
                    // A cell put is the cell's whole look: what the overlay
                    // held there before is gone.
                    model.overlay_each(rect, |offset, cell| {
                        *cell = OverlayCell { character: characters.get(offset).copied(), ink, paper, flash };
                    });
                    Ok(())
                })?;
            }
            Op::TT_OVERLAY_FILL => {
                let [row, column, rows, columns, ink, paper] = exactly(op, ints)?;
                no_payload(op, payload)?;
                let rect = rect_of(row, column, rows, columns)?;
                let (ink, paper) = (entry(ink)?, entry(paper)?);
                self.change(|model| {
                    // Colours only: each cell keeps its character and flash.
                    model.overlay_each(rect, |_, cell| {
                        cell.ink = ink.or(cell.ink);
                        cell.paper = paper.or(cell.paper);
                    });
                    Ok(())
                })?;
            }
            _ => {
                let [row, column, rows, columns] = exactly(op, ints)?;
                no_payload(op, payload)?;
                // No rows: the whole overlay (section 10.2).
                let rect = if rows == 0 { None } else { Some(rect_of(row, column, rows, columns)?) };
                self.change(|model| {
                    match rect {
                        Some(rect) => model.overlay_each(rect, |_, cell| *cell = OverlayCell::default()),
                        None => model.overlay_clear(),
                    }
                    Ok(())
                })?;
            }
        }
        Ok(Reply::Undefined)
    }

    /// The place among the controls of the one `handle` names, or the
    /// `TypeError` of a stale or forged handle (section 10.1 rule 4).
    fn control_of(&self, handle: i32) -> Result<usize, Reply> {
        control_cell(handle).and_then(|(row, column)| self.model.control_at(row, column)).ok_or_else(|| type_error("no such thing"))
    }

    /// `control(...)` (section 5.4): a new control, or with `joined` one
    /// more row of the control that ends on the row above.
    fn control(&mut self, op: Op, ints: &[i32], payload: Payload<'_>, scope: i32) -> Result<Reply, Reply> {
        let [row, column, columns, joined] = exactly(op, ints)?;
        no_payload(op, payload)?;
        let (line, at) = (row_of(row)?, column_of(column)?);
        let width = within(columns, 1, LAST_COLUMN.saturating_sub(column).saturating_add(1), "COLUMNS", "the columns of a control from this column")?;
        if !(0..=1).contains(&joined) {
            return Err(type_error("joined is true or false"));
        }
        if self.model.control_over(line, at, at.saturating_add(width)) {
            return Err(type_error("a control already holds one of those cells"));
        }
        let above = if joined == 1 {
            Some(
                self.model
                    .control_above(line, at, width)
                    .ok_or_else(|| type_error("a joined control continues one that ends on the row above at the same column and width"))?,
            )
        } else {
            if self.model.controls.len() >= MAX_CONTROLS {
                return Err(range("MAX_CONTROLS", &format!("a page has at most {MAX_CONTROLS} controls")));
            }
            None
        };
        let first = self.change(|model| {
            if let Some(place) = above {
                let control = model.controls.get_mut(place).ok_or_else(|| type_error("no such thing"))?;
                control.height = control.height.saturating_add(1);
                Ok((control.row, control.column))
            } else {
                model.controls.push(Control { row: line, column: at, width, height: 1, look: 0, scope });
                Ok((line, at))
            }
        })?;
        control_handle(first.0, first.1).map(Reply::Int).ok_or_else(|| type_error("no such thing"))
    }

    /// `TT_LABEL`, `TT_ANIMATE` and `TT_REMOVE` for a control (sections
    /// 5.4 and 3.9). Sprites, which share these operations, are work
    /// package T2's.
    fn thing(&mut self, op: Op, ints: &[i32], payload: Payload<'_>) -> Result<Reply, Reply> {
        match op {
            Op::TT_LABEL => {
                let [handle] = exactly(op, ints)?;
                let label = text_of(op, payload)?;
                self.control_of(handle)?;
                if label.len() > MAX_LABEL_BYTES {
                    return Err(range("MAX_LABEL_BYTES", &format!("a label is at most {MAX_LABEL_BYTES} bytes of UTF-8")));
                }
                if label.chars().any(char::is_control) {
                    return Err(type_error("a label holds no control character"));
                }
                self.labels.retain(|staged| staged.handle != handle);
                self.labels.push(Label { handle, label: label.to_owned() });
            }
            Op::TT_ANIMATE => {
                let [handle, operation, look, second, third, fourth] = exactly(op, ints)?;
                no_payload(op, payload)?;
                let place = self.control_of(handle)?;
                if operation != i32::from(code::LOOK) || (second, third, fourth) != (0, 0, 0) {
                    return Err(type_error("a control takes the animation operation look alone"));
                }
                let look = within(look, 0, LAST_CONTROL_LOOK, "MAX_LOOK", "a control's look")?;
                if let Some(control) = self.model.controls.get_mut(place) {
                    control.look = look;
                }
            }
            _ => {
                let [handle] = exactly(op, ints)?;
                no_payload(op, payload)?;
                let place = self.control_of(handle)?;
                self.labels.retain(|staged| staged.handle != handle);
                self.change(|model| {
                    model.controls.remove(place);
                    Ok(())
                })?;
            }
        }
        Ok(Reply::Undefined)
    }

    /// One operation of the page context.
    fn operate(&mut self, op: Op, ints: &[i32], payload: Payload<'_>, scope: i32, tick: u64) -> Result<Reply, Reply> {
        // The page plane writes no row the sixel bitmap owns (section 5.7).
        if let Some(why) = self.bitmap.refuses(op, ints) {
            return Err(Reply::Type(why));
        }
        match op {
            Op::BM_ROWS | Op::BM_CLEAR | Op::BM_SET | Op::BM_GET | Op::BM_LINE | Op::BM_RECT | Op::BM_BLIT | Op::BM_TEXT | Op::BM_PRESENT => {
                self.bitmap_operate(op, ints, payload)
            }
            Op::TT_PUT => self.put(op, ints, payload),
            Op::TT_CODES | Op::TT_BLIT | Op::TT_AT | Op::TT_CLEAR | Op::TT_SCROLL | Op::TT_SETTINGS => self.plane(op, ints, payload),
            Op::TT_OVERLAY_PUT | Op::TT_OVERLAY_FILL | Op::TT_OVERLAY_CLEAR => self.overlay(op, ints, payload),
            Op::TT_COLOUR_DEFINE => {
                let [colour] = exactly(op, ints)?;
                // The name is the script's own word for the colour. The
                // budget report counts colours and names none, so the
                // name is checked as any label is and not kept.
                let name = text_of(op, payload)?;
                if name.len() > MAX_LABEL_BYTES {
                    return Err(range("MAX_LABEL_BYTES", &format!("a colour's name is at most {MAX_LABEL_BYTES} bytes of UTF-8")));
                }
                self.colours.define(colour, tick).map(Reply::Int)
            }
            Op::TT_COLOUR_SET => {
                let [entry, colour] = exactly(op, ints)?;
                no_payload(op, payload)?;
                self.colours.set(entry, colour, tick).map(|()| Reply::Undefined)
            }
            Op::TT_CONTROL => self.control(op, ints, payload, scope),
            Op::TT_LABEL | Op::TT_ANIMATE | Op::TT_REMOVE => self.thing(op, ints, payload),
            _ => Err(Reply::Type(format!("operation {op:?} is not the teletext page's"))),
        }
    }

    /// What every context may ask: the level, the languages, and a text's
    /// measure (section 5.6).
    fn ask(&self, op: Op, ints: &[i32], payload: Payload<'_>) -> Option<Result<Reply, Reply>> {
        Some(match op {
            Op::TT_LEVEL => exactly::<0>(op, ints).and_then(|[]| no_payload(op, payload)).map(|()| Reply::Int(i32::from(self.level))),
            Op::TT_LANGUAGES => exactly::<0>(op, ints).and_then(|[]| no_payload(op, payload)).map(|()| Reply::Text(self.languages.json())),
            Op::TT_MEASURE => exactly(op, ints).and_then(|[language]| {
                let cells = self.languages.measure(text_of(op, payload)?, language)?;
                Ok(Reply::Int(i32::try_from(cells).unwrap_or(i32::MAX)))
            }),
            _ => return None,
        })
    }

    /// The step's changes, as the batch's sections; what the decoder has
    /// been told is then what the page is.
    fn fill(&mut self, batch: &mut Batch) {
        let mut rows = Vec::new();
        for ((row, cells), told) in (limits::FIRST_ROW_BYTE..).zip(&self.model.plane).zip(self.rows_told.iter_mut()) {
            let mut codes = [SPACE; COLUMNS];
            for (code, cell) in codes.iter_mut().zip(cells) {
                *code = cell.code;
            }
            if codes != *told {
                *told = codes;
                rows.push(TeletextRow { row, codes });
            }
        }
        if !rows.is_empty() {
            batch.teletext_rows = Some(rows);
        }
        let stream = self.model.stream(self.languages.page_value());
        if stream != self.stream_told {
            self.stream_told.clone_from(&stream);
            batch.teletext_enhancement = Some(stream);
        }
        let PageSettings { screen, row_colour, substitution } = self.model.settings;
        let settings = TeletextSettings { screen_colour: screen, row_colour, substitution, colours: self.colours.shown() };
        if settings != self.settings_told {
            self.settings_told.clone_from(&settings);
            batch.teletext_settings = Some(settings);
        }
        // The engine keeps a forced look by the control's number, and
        // numbers controls in the order their triplets come: a control
        // made or removed renumbers those after it. So what is told is
        // the look each number is to have, wherever that differs from
        // what the number was last told.
        let mut looks = [0_u8; MAX_CONTROLS];
        for (look, control) in looks.iter_mut().zip(self.model.controls_in_order()) {
            *look = control.look;
        }
        let mut operations = Vec::new();
        for ((number, look), told) in (1_u16..).zip(looks).zip(self.looks_told.iter_mut()) {
            if look != *told {
                *told = look;
                operations.push(Operation { target: Target::control(number), kind: OperationKind::Look(look) });
            }
        }
        if !operations.is_empty() {
            batch.animate.get_or_insert_default().append(&mut operations);
        }
        if !self.labels.is_empty() {
            batch.labels.get_or_insert_default().append(&mut self.labels);
        }
    }
}

/// The teletext state of a container's host: there while its page context
/// is open on a teletext page, and gone with that context, since a page's
/// renderer state belongs to the page (section 3.1).
#[derive(Default)]
pub struct Slot(Option<Box<Teletext>>);

impl Slot {
    /// A context opens: the page context of a teletext page starts a
    /// blank page.
    pub fn open(&mut self, open: &ContextOpen) {
        if open.context == ContextId::Page {
            self.0 = (open.renderer == Renderer::Teletext).then(|| Teletext::new(open));
        }
    }

    /// A context closes: the page's state goes with the page context.
    pub fn close(&mut self, context: ContextId) {
        if context == ContextId::Page {
            self.0 = None;
        }
    }

    /// A step begins at the presentation tick `tick`: colours that waited
    /// for their rate are applied.
    pub fn begin_step(&mut self, tick: u64) {
        if let Some(held) = self.0.as_mut() {
            held.colours.begin_step(tick);
        }
    }

    /// The scope `scope` of `context` ends: its controls are removed, and
    /// their handles given.
    pub fn scope_end(&mut self, context: ContextId, scope: i32) -> Vec<i32> {
        let (ContextId::Page, Some(held)) = (context, self.0.as_mut()) else { return Vec::new() };
        let handles: Vec<i32> =
            held.model.controls.iter().filter(|control| control.scope == scope).filter_map(|control| control_handle(control.row, control.column)).collect();
        if !handles.is_empty() {
            held.labels.retain(|staged| !handles.contains(&staged.handle));
            held.model.controls.retain(|control| control.scope != scope);
            held.used = held.model.stream(held.languages.page_value()).len();
        }
        handles
    }

    /// Whether `handle` names a thing of `context`: what operation
    /// `LISTEN` accepts beside the shared core's two.
    #[must_use]
    pub fn has_thing(&self, context: ContextId, handle: i32) -> bool {
        context == ContextId::Page && self.0.as_ref().is_some_and(|held| held.control_of(handle).is_ok())
    }

    /// The bytes held for the page.
    #[must_use]
    pub fn state_bytes(&self) -> usize {
        self.0.as_ref().map_or(0, |held| held.state_bytes())
    }

    /// The bytes of the step's output that grow with what a script does:
    /// its labels. The other sections are fixed in size, a page's rows,
    /// triplets and settings at most, and are counted in what is held.
    #[must_use]
    pub fn batch_bytes(&self) -> usize {
        self.0.as_ref().map_or(0, |held| held.batch_bytes())
    }

    /// The tick the page next needs a step at with no input: when a
    /// colour that waits for its rate can be applied.
    #[must_use]
    pub fn next_due(&self, tick: u64) -> Option<u64> {
        self.0.as_ref().and_then(|held| held.colours.next_due(tick))
    }

    /// Adds the step's teletext sections to `batch`.
    pub fn fill(&mut self, batch: &mut Batch) {
        if let Some(held) = self.0.as_mut() {
            held.fill(batch);
        }
    }

    /// The teletext entries of the budget report (section 3.7), each
    /// written after a comma.
    pub fn budget(&self, out: &mut String) {
        if let Some(held) = self.0.as_ref() {
            let _ = write!(
                out,
                ",\"triplets\":{{\"used\":{},\"limit\":{LOCAL_TRIPLETS}}},\"colours\":{{\"used\":{},\"limit\":{TELETEXT_DEFINABLE_COLOURS}}},\"controls\":{{\"used\":{},\"limit\":{MAX_CONTROLS}}}",
                held.used,
                held.colours.defined(),
                held.model.controls.len(),
            );
            let _ = write!(out, ",\"bitmapCellsLossy\":{{\"used\":{},\"limit\":{}}}", held.bitmap.lossy(), bitmap::CELLS);
        }
    }

    /// One teletext operation of `context`, at the presentation tick
    /// `tick`, with `scope` the scope new things belong to (0 for none).
    ///
    /// # Errors
    /// The `RangeError` or `TypeError` the script is thrown; nothing has
    /// then changed.
    pub fn call(&mut self, context: ContextId, op: Op, ints: &[i32], payload: Payload<'_>, scope: i32, tick: u64) -> Result<Reply, Reply> {
        let Some(held) = self.0.as_mut() else {
            return Err(type_error("the page shown is not a teletext page with a script of its own"));
        };
        if let Some(reply) = held.ask(op, ints, payload) {
            return reply;
        }
        // Renderer state belongs to the page (section 3.1): the plane, the
        // overlay, the Colour Map and the controls are the page context's.
        if context != ContextId::Page {
            return Err(type_error("only the page context draws on the page plane, the overlay, the Colour Map and the controls"));
        }
        held.operate(op, ints, payload, scope, tick)
    }
}

#[cfg(test)]
mod tests;
