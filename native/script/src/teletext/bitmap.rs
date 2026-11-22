//! The sixel bitmap's host, `nt.teletext.bitmap` (script-api.md section
//! 5.7, operations 130 to 138 of section 10.2; the owner's DEC-7245:
//! "teletext must also be programmable as a double buffered sixel bitmap
//! which gets transformed into characters"; work package T3).
//!
//! The host keeps the two buffers. The **back buffer** is the script's
//! own: [`BITMAP_WIDTH`] by [`BITMAP_HEIGHT`] sixels, one Colour Map entry
//! each, with the cells `bitmap.text` reserved for characters. Every
//! drawing call writes it and nothing else, so nothing a script draws is
//! seen until it calls `present()`. The **frame** is what is shown: the
//! engine's [`BitmapFrame`], which holds the Level 1 codes, each cell as a
//! receiver draws it and the X/26 column triplets of the rows the bitmap
//! owns.
//!
//! `present()` gives both to the engine's one converter
//! (`neoteletext_picture::bitmap::present`, the engine's DEC-7355), so
//! that the host converts nothing itself, and writes the new frame to the
//! page the rest of the teletext API draws on, through
//! [`Teletext::change`]: the frame's codes become the codes of the page
//! plane's rows, its character triplets the marks of those cells, and its
//! colour triplets the colours under the overlay ([`Colours`]), from which
//! the page's local enhancement data is written whole, in row and column
//! order, with the overlay's and the controls' triplets
//! (`Model::stream`). The bitmap's triplets are therefore paid for from
//! the same [`LOCAL_TRIPLETS`] as everything else on the page.
//!
//! A script's arguments are outside input: every colour, coordinate, row
//! and length is checked before a buffer is touched, the work of each call
//! is bounded by the bitmap and by [`MAX_BITMAP_COORDINATE`], and both
//! buffers are fixed in size (section 10.1 rules 1 to 3 and 7).

use neoteletext_core::consts::{
    COLOUR_BLACK, COLOUR_MAP_DEFAULTS, COLOUR_MAP_ENTRIES, COLOUR_WHITE, COLUMNS, MODE_BACKGROUND_COLOUR, MODE_FOREGROUND_COLOUR, SPACE,
    TRIPLET_COLOUR_INDEX_MASK,
};
use neoteletext_picture::Level;
use neoteletext_picture::bitmap::{
    BITMAP_CELLS, BITMAP_FIRST_ROW, BITMAP_HEIGHT, BITMAP_LAST_ROW, BITMAP_MAX_TRIPLETS, BITMAP_ROWS, BITMAP_SIXELS, BITMAP_WIDTH, BitmapFrame, BitmapInput,
    BitmapText, Presented, is_bitmap_colour, present,
};
use protocol::script::Level as LinkLevel;
use protocol::script::batch::MAX_ROW_CODE;

use super::limits::{FIRST_ROW, LAST_COLUMN, LAST_ROW, LOCAL_TRIPLETS, PLANE_ROWS};
use super::model::{Model, PlaneCell};
use super::{NONE, Teletext, bytes_of, exactly, no_payload, range, text_of, type_error, within};
use crate::host::{Payload, Reply};
use crate::ops::Op;

/// The script-side half of the bitmap, which comes before the teletext
/// API's in the prelude so that the API holds it before it is frozen.
pub const PRELUDE: &str = include_str!("bitmap.js");

/// Sixels across a cell and down it (EN 300 706 clause 15.6.1: a block
/// mosaic is two by three).
const SIXELS_ACROSS: usize = BITMAP_WIDTH / COLUMNS;
const SIXELS_DOWN: usize = BITMAP_HEIGHT / BITMAP_ROWS;

/// `value` as a script's whole number, at compile time; a constant that
/// did not fit would fail the build here.
const fn wide(value: usize) -> i32 {
    assert!(value <= i32::MAX as usize);
    #[expect(clippy::cast_possible_truncation, clippy::cast_possible_wrap, reason = "the assertion above holds it to 31 bits")]
    let wide = value as i32;
    wide
}

/// The bitmap's sides as a script's whole numbers.
const WIDTH: i32 = wide(BITMAP_WIDTH);
const HEIGHT: i32 = wide(BITMAP_HEIGHT);

/// The furthest a coordinate, a width or a height may lie from the
/// bitmap's origin: twelve bitmaps' width. Section 5.7 says "coordinates
/// outside are clipped" and names no bound; without one a line between
/// the ends of the whole-number range would be four thousand million
/// steps of host work (section 10.1 rule 2). With it a line is at most
/// 2,049 steps.
pub const MAX_BITMAP_COORDINATE: i32 = 1024;

/// The byte of `bitmap.blit`'s data that leaves a sixel as it is, unless
/// the script names another (section 5.7).
const DEFAULT_TRANSPARENT: i32 = 255;
/// The ink and paper of `bitmap.text` when the script names none: white
/// on black, as a row starts (EN 300 706 clause 12.2).
const DEFAULT_INK: u8 = COLOUR_WHITE;
const DEFAULT_PAPER: u8 = COLOUR_BLACK;

/// Where `present()`'s second number lies in its result: the cells
/// changed, plus this many times the cells that lost detail (section
/// 10.2, operation 138).
pub const PRESENT_LOSSY_FACTOR: i32 = 65_536;

/// The most times one `present()` asks the engine to convert the bitmap.
/// The engine is told how many triplets the page has left and keeps to
/// them, but the overlay may have to state a colour again after a cell of
/// the bitmap's, which only writing the page's enhancement data shows. So
/// a frame that does not fit is converted again with that many fewer
/// triplets, and a last time with none, which is the Level 1 rule alone.
const PRESENT_ATTEMPTS: usize = 3;

// The bitmap is the page plane's rows and cells, sixel for sixel, its
// result's two numbers cannot run into each other, and the engine never
// writes more column triplets than the page could hold.
const _: () = assert!(wide(BITMAP_FIRST_ROW) == FIRST_ROW && wide(BITMAP_LAST_ROW) == LAST_ROW && BITMAP_ROWS == PLANE_ROWS);
const _: () = assert!(BITMAP_SIXELS == BITMAP_WIDTH * BITMAP_HEIGHT && BITMAP_CELLS == PLANE_ROWS * COLUMNS);
const _: () = assert!(SIXELS_ACROSS * COLUMNS == BITMAP_WIDTH && SIXELS_DOWN * BITMAP_ROWS == BITMAP_HEIGHT);
const _: () = assert!(wide(BITMAP_CELLS) < PRESENT_LOSSY_FACTOR && wide(BITMAP_CELLS) <= i32::MAX / (PRESENT_LOSSY_FACTOR + 1));
const _: () = assert!(BITMAP_MAX_TRIPLETS < LOCAL_TRIPLETS);
const _: () = assert!(MAX_BITMAP_COORDINATE >= WIDTH && MAX_BITMAP_COORDINATE >= HEIGHT);

/// The cells of the bitmap: the most `present()` can count as changed or
/// as having lost detail, and the limit of the budget report's
/// `bitmapCellsLossy`.
pub const CELLS: usize = BITMAP_CELLS;

/// The numbers the prelude's bitmap part shares with this file, written
/// in front of the prelude as the table `TTB` (section 10.1: defined
/// once, in Rust).
#[must_use]
pub fn prelude_numbers() -> [(&'static str, i64); 8] {
    [
        ("WIDTH", i64::from(WIDTH)),
        ("HEIGHT", i64::from(HEIGHT)),
        ("FIRST_ROW", i64::from(FIRST_ROW)),
        ("LAST_ROW", i64::from(LAST_ROW)),
        ("NO_ROWS", i64::from(NO_ROWS)),
        ("TRANSPARENT", i64::from(DEFAULT_TRANSPARENT)),
        ("NONE", i64::from(NONE)),
        ("LOSSY_FACTOR", i64::from(PRESENT_LOSSY_FACTOR)),
    ]
}

/// Operation `BM_ROWS`'s word for "no rows": the bitmap gives every row
/// back to `page` (`bitmap.release()`). Row 0 is the receiver's, so it
/// names no row of the bitmap's.
const NO_ROWS: i32 = 0;

/// The colours the bitmap's own X/26 colour triplets give one cell.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct Under {
    /// A foreground colour triplet at the cell: a Colour Map entry.
    ink: Option<u8>,
    /// A background colour triplet at the cell.
    paper: Option<u8>,
}

/// The colours the bitmap's frame sets by non-spacing attributes, cell by
/// cell: part of the page's model, so that the writer of the page's local
/// enhancement data takes them as the look under the overlay and writes
/// one stream in row and column order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Colours([[Under; COLUMNS]; PLANE_ROWS]);

impl Default for Colours {
    fn default() -> Self {
        Self([[Under::default(); COLUMNS]; PLANE_ROWS])
    }
}

/// The place of display row `row` among the plane's rows.
fn index(row: u8) -> Option<usize> {
    usize::from(row).checked_sub(BITMAP_FIRST_ROW)
}

impl Colours {
    /// The foreground and the background the bitmap's triplets set at
    /// `row`, `column`; none where it sets none.
    #[must_use]
    pub fn at(&self, row: u8, column: u8) -> (Option<u8>, Option<u8>) {
        let under = index(row).and_then(|line| self.0.get(line)).and_then(|line| line.get(usize::from(column))).copied().unwrap_or_default();
        (under.ink, under.paper)
    }

    /// Forgets the colours of `row`.
    fn clear_row(&mut self, row: u8) {
        if let Some(line) = index(row).and_then(|line| self.0.get_mut(line)) {
            *line = [Under::default(); COLUMNS];
        }
    }
}

/// What the host holds for the bitmap: fixed in size.
pub struct Bitmap {
    /// The back buffer: each sixel's colour, [`BITMAP_WIDTH`] a line from
    /// the top left of row 1.
    back: [u8; BITMAP_SIXELS],
    /// The cells of the back buffer reserved for characters.
    text: [BitmapText; BITMAP_CELLS],
    /// What is shown.
    frame: BitmapFrame,
    /// The rows the bitmap owns, first and last; none until the script
    /// takes some.
    rows: Option<(u8, u8)>,
    /// The cells of the last `present()` that lost detail.
    lossy: usize,
}

impl Bitmap {
    /// Two black buffers, no text and no rows.
    #[must_use]
    pub fn new() -> Box<Self> {
        Box::new(Self { back: [COLOUR_BLACK; BITMAP_SIXELS], text: [BitmapText::NONE; BITMAP_CELLS], frame: BitmapFrame::BLANK, rows: None, lossy: 0 })
    }

    /// Whether the bitmap owns `row`.
    fn owns(&self, row: i32) -> bool {
        self.rows.is_some_and(|(first, last)| (i32::from(first)..=i32::from(last)).contains(&row))
    }

    /// Whether the bitmap owns any of the rows `first` to `last`.
    fn owns_any(&self, first: i32, last: i32) -> bool {
        self.rows.is_some_and(|(top, bottom)| first <= i32::from(bottom) && i32::from(top) <= last)
    }

    /// The cells of the last `present()` that lost detail, for the budget
    /// report's `bitmapCellsLossy` (section 3.7).
    #[must_use]
    pub const fn lossy(&self) -> usize {
        self.lossy
    }

    /// Why the page plane's operation `op` is refused while the bitmap
    /// owns the rows it would write, or none (section 5.7: "the bitmap
    /// and `page` write the same Level 1 plane, each on the rows it
    /// owns"). The frame must stay what the plane's rows hold, so every
    /// operation that writes a row is refused, not only the three section
    /// 5.7 names; and black background colour substitution stays off,
    /// since the engine encodes each row from a black start (DEC-7355).
    #[must_use]
    pub fn refuses(&self, op: Op, ints: &[i32]) -> Option<String> {
        let (first, last) = self.rows?;
        let number = |at: usize| ints.get(at).copied();
        let refused = match op {
            // Double height hides the row below, which would be the
            // bitmap's.
            Op::TT_PUT => number(0).is_some_and(|row| self.owns(row) || (number(4) == Some(2) && self.owns(row.saturating_add(1)))),
            Op::TT_CODES => number(0).is_some_and(|row| self.owns(row)),
            Op::TT_BLIT => true,
            Op::TT_CLEAR | Op::TT_SCROLL => matches!((number(0), number(1)), (Some(top), Some(bottom)) if self.owns_any(top, bottom)),
            Op::TT_SETTINGS => {
                return (number(2) == Some(1)).then(|| {
                    format!(
                        "black background colour substitution stays off while the bitmap owns rows {first} to {last}: its rows are encoded from a black start"
                    )
                });
            }
            _ => false,
        };
        refused.then(|| format!("rows {first} to {last} are the bitmap's: the page plane writes the other rows, and text there is bitmap.text"))
    }

    /// The sixel at (`x`, `y`) of the back buffer, where that is inside.
    fn sixel_mut(&mut self, x: i32, y: i32) -> Option<&mut u8> {
        let (across, down) = (usize::try_from(x).ok().filter(|across| *across < BITMAP_WIDTH)?, usize::try_from(y).ok()?);
        self.back.get_mut(down.checked_mul(BITMAP_WIDTH)?.checked_add(across)?)
    }

    /// Draws the sixel at (`x`, `y`) in `colour`; outside the bitmap it
    /// is clipped.
    fn plot(&mut self, x: i32, y: i32, colour: u8) {
        if let Some(sixel) = self.sixel_mut(x, y) {
            *sixel = colour;
        }
    }

    /// Fills the sixels of the rectangle's part inside the bitmap: `x`
    /// from `left` to before `right`, `y` from `top` to before `bottom`.
    fn fill(&mut self, left: i32, top: i32, right: i32, bottom: i32, colour: u8) {
        for y in top.max(0)..bottom.min(HEIGHT) {
            for x in left.max(0)..right.min(WIDTH) {
                self.plot(x, y, colour);
            }
        }
    }

    /// A line from (`x0`, `y0`) to (`x1`, `y1`), both ends drawn, by
    /// Bresenham's algorithm in whole numbers: at most twice
    /// [`MAX_BITMAP_COORDINATE`] and one steps, the caller having held
    /// the ends to that bound.
    fn line(&mut self, (x0, y0): (i32, i32), (x1, y1): (i32, i32), colour: u8) {
        let (across, down) = (x1.saturating_sub(x0).saturating_abs(), y1.saturating_sub(y0).saturating_abs());
        let (step_x, step_y) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
        let (mut x, mut y) = (x0, y0);
        let mut error = across.saturating_sub(down);
        for _ in 0..=across.max(down) {
            self.plot(x, y, colour);
            let doubled = error.saturating_mul(2);
            if doubled > down.saturating_neg() {
                error = error.saturating_sub(down);
                x = x.saturating_add(step_x);
            }
            if doubled < across {
                error = error.saturating_add(across);
                y = y.saturating_add(step_y);
            }
        }
    }
}

/// A sixel's or a text cell's colour: 0 to 7, or an entry 16 to 31
/// (section 5.7, "Colours").
fn colour_of(colour: i32, what: &str) -> Result<u8, Reply> {
    u8::try_from(colour)
        .ok()
        .filter(|colour| is_bitmap_colour(*colour))
        .ok_or_else(|| range("BITMAP_COLOURS", &format!("{what} is 0 to 7 or a Colour Map entry 16 to 31, not {colour}")))
}

/// A coordinate, a width or a height within [`MAX_BITMAP_COORDINATE`] of
/// the origin.
fn coordinate_of(value: i32, what: &str) -> Result<i32, Reply> {
    if (MAX_BITMAP_COORDINATE.saturating_neg()..=MAX_BITMAP_COORDINATE).contains(&value) {
        Ok(value)
    } else {
        Err(range("MAX_BITMAP_COORDINATE", &format!("{what} is within {MAX_BITMAP_COORDINATE} of the bitmap's origin, not {value}")))
    }
}

/// A width or a height: 0 to [`MAX_BITMAP_COORDINATE`].
fn extent_of(value: i32, what: &str) -> Result<i32, Reply> {
    if value < 0 { Err(range("MAX_BITMAP_COORDINATE", &format!("{what} is 0 to {MAX_BITMAP_COORDINATE}, not {value}"))) } else { coordinate_of(value, what) }
}

/// The engine's name for the receiver's presentation level. A code that
/// is no level's cannot come from a context's opening; it is taken as
/// Level 1, which every receiver shows.
fn level_of(code: u8) -> Level {
    match LinkLevel::of(code) {
        Ok(LinkLevel::OneFive) => Level::OnePointFive,
        Ok(LinkLevel::TwoFive) => Level::TwoPointFive,
        Ok(LinkLevel::ThreeFive) => Level::ThreePointFive,
        Ok(LinkLevel::One) | Err(_) => Level::One,
    }
}

/// Gives `row` to the bitmap, or back to `page`: a row of spaces in the
/// plane with no colours of the bitmap's, and the same in the frame (the
/// engine's DEC-7355: the frame must match the plane on every row the
/// bitmap owns).
fn hand_over(model: &mut Model, frame: &mut BitmapFrame, row: u8) {
    model.clear_rows(row, row);
    model.bitmap.clear_row(row);
    frame.reset_row(usize::from(row));
}

/// Writes the rows `first` to `last` of `frame` to `model`: each row's
/// Level 1 codes to the plane, each character triplet as the mark of its
/// cell, and each colour triplet as the cell's colour under the overlay.
fn show(model: &mut Model, frame: &BitmapFrame, first: u8, last: u8) {
    for row in first..=last {
        let Some(codes) = frame.row(usize::from(row)) else { continue };
        model.write(row, 0, codes.iter().map(|code| PlaneCell::code(*code)));
        model.bitmap.clear_row(row);
    }
    for triplet in frame.triplets() {
        let Some(line) = index(triplet.row) else { continue };
        let column = usize::from(triplet.column);
        let under = model.bitmap.0.get_mut(line).and_then(|cells| cells.get_mut(column));
        let cell = model.plane.get_mut(line).and_then(|cells| cells.get_mut(column));
        match (triplet.mode, under, cell) {
            (MODE_FOREGROUND_COLOUR, Some(under), _) => under.ink = Some(triplet.data & TRIPLET_COLOUR_INDEX_MASK),
            (MODE_BACKGROUND_COLOUR, Some(under), _) => under.paper = Some(triplet.data & TRIPLET_COLOUR_INDEX_MASK),
            // A mosaic or a character the row's own code cannot show
            // (the engine's modes G1 and G0 without a diacritical mark).
            (mode, _, Some(cell)) => cell.mark = Some((mode, triplet.data)),
            _ => {}
        }
    }
}

impl Teletext {
    /// The Colour Map as the page has it now: the standard's defaults
    /// (EN 300 706 clause 12.4, Table 30) with the entries the script
    /// has defined as they are shown, a colour that waits for its rate
    /// not yet among them.
    fn colour_map(&self) -> [u16; COLOUR_MAP_ENTRIES] {
        let mut map = COLOUR_MAP_DEFAULTS;
        for (entry, colour) in self.colours.shown() {
            if let Some(slot) = map.get_mut(usize::from(entry)) {
                *slot = colour;
            }
        }
        map
    }

    /// `bitmap.rows(first, last)` and `bitmap.release()` (section 5.7).
    /// A row that changes owner is cleared, on the page and in the frame,
    /// so the change shows at once; the back buffer is not touched.
    fn bitmap_rows(&mut self, first: i32, last: i32) -> Result<(), Reply> {
        let wanted = if (first, last) == (NO_ROWS, NO_ROWS) {
            None
        } else {
            let (top, bottom) = (within(first, FIRST_ROW, LAST_ROW, "TELETEXT_ROWS", "a row")?, within(last, FIRST_ROW, LAST_ROW, "TELETEXT_ROWS", "a row")?);
            if top > bottom {
                return Err(range("TELETEXT_ROWS", &format!("the first row, {first}, is after the last, {last}")));
            }
            Some((top, bottom))
        };
        if wanted.is_some() && self.model.settings.substitution {
            return Err(type_error(
                "the bitmap owns no row while black background colour substitution is on: its rows are encoded from a black start; turn it off with page.settings",
            ));
        }
        let held = self.bitmap.rows;
        let inside = |rows: Option<(u8, u8)>, row: u8| rows.is_some_and(|(top, bottom)| (top..=bottom).contains(&row));
        let mut frame = self.bitmap.frame.clone();
        self.change(|model| {
            for row in super::limits::FIRST_ROW_BYTE..=u8::try_from(LAST_ROW).unwrap_or(u8::MAX) {
                if inside(held, row) != inside(wanted, row) {
                    hand_over(model, &mut frame, row);
                }
            }
            Ok(())
        })?;
        self.bitmap.frame = frame;
        self.bitmap.rows = wanted;
        Ok(())
    }

    /// `bitmap.text` (section 5.7): reserves the cells of `text` from
    /// `column` of `row` in the back buffer. A character of a bitmap's
    /// row is drawn by the row's own code, so it is one the page's own G0
    /// set shows with no mark and no triplet of its own.
    fn bitmap_text(&mut self, op: Op, ints: &[i32], payload: Payload<'_>) -> Result<(), Reply> {
        let [row, column, ink, paper] = exactly(op, ints)?;
        let text = text_of(op, payload)?;
        let line = within(row, FIRST_ROW, LAST_ROW, "TELETEXT_ROWS", "a row")?;
        let at = within(column, 0, LAST_COLUMN, "COLUMNS", "a column")?;
        let ink = if ink == NONE { DEFAULT_INK } else { colour_of(ink, "a text's ink")? };
        let paper = if paper == NONE { DEFAULT_PAPER } else { colour_of(paper, "a text's paper")? };
        let cells = self.languages.plane(line, at, text, NONE, &[])?;
        if let Some(cell) = cells.iter().find(|cell| cell.mark.is_some() || cell.set.is_some() || !(SPACE..=MAX_ROW_CODE).contains(&cell.code)) {
            return Err(type_error(&format!(
                "a bitmap's text is characters of the page's own G0 set, with no diacritical mark, no G2 character and no other language; the code {} is not",
                cell.code
            )));
        }
        let start = index(line).and_then(|line| line.checked_mul(COLUMNS)).and_then(|start| start.checked_add(usize::from(at))).unwrap_or(BITMAP_CELLS);
        for (slot, cell) in self.bitmap.text.iter_mut().skip(start).zip(&cells) {
            *slot = BitmapText { code: cell.code, ink, paper };
        }
        Ok(())
    }

    /// `bitmap.blit` (section 5.7): `width` by `height` bytes, one a
    /// sixel, drawn from (`x`, `y`); a byte equal to `transparent` leaves
    /// its sixel as it is, and every other byte is a colour, checked
    /// before any is drawn.
    fn bitmap_blit(&mut self, op: Op, ints: &[i32], payload: Payload<'_>) -> Result<(), Reply> {
        let [x, y, width, height, transparent] = exactly(op, ints)?;
        let data = bytes_of(op, payload)?;
        let (x, y) = (coordinate_of(x, "x")?, coordinate_of(y, "y")?);
        let (width, height) = (extent_of(width, "a width")?, extent_of(height, "a height")?);
        let transparent = within(transparent, 0, i32::from(u8::MAX), "BITMAP_BLIT", "the transparent byte")?;
        let across = usize::try_from(width).unwrap_or(0);
        let wanted = across.saturating_mul(usize::try_from(height).unwrap_or(0));
        if data.len() != wanted {
            return Err(range("BITMAP_BLIT", &format!("{width} by {height} sixels are exactly {wanted} bytes, not {}", data.len())));
        }
        if let Some(byte) = data.iter().find(|byte| **byte != transparent && !is_bitmap_colour(**byte)) {
            return Err(range("BITMAP_COLOURS", &format!("a sixel's colour is 0 to 7 or a Colour Map entry 16 to 31, not {byte}")));
        }
        if across == 0 {
            return Ok(());
        }
        for (down, line) in (y..).zip(data.chunks(across)) {
            for (at, byte) in (x..).zip(line) {
                if *byte != transparent {
                    self.bitmap.plot(at, down, *byte);
                }
            }
        }
        Ok(())
    }

    /// `bitmap.present()` (section 5.7): converts the back buffer to
    /// characters, makes them what the page shows on the rows the bitmap
    /// owns, and gives the cells that changed and the cells that lost
    /// detail as the engine counts them. Only rows whose codes changed
    /// and, if any triplet changed, the page's enhancement data leave in
    /// the step's batch, which is the teletext API's own rule
    /// ([`Teletext::fill`]).
    ///
    /// A bitmap that was given no rows takes rows 1 to 23 here, section
    /// 5.7's default.
    fn bitmap_present(&mut self) -> Result<Reply, Reply> {
        let first_present = self.bitmap.rows.is_none();
        if first_present && self.model.settings.substitution {
            return Err(type_error(
                "the bitmap owns no row while black background colour substitution is on: its rows are encoded from a black start; turn it off with page.settings",
            ));
        }
        let (first, last) = self.bitmap.rows.unwrap_or((super::limits::FIRST_ROW_BYTE, u8::try_from(LAST_ROW).unwrap_or(u8::MAX)));
        // The page with the bitmap's rows empty: what everything else on
        // the page needs of the page's triplets, the Termination Marker
        // included, is what the bitmap cannot have.
        let mut cleared = self.model.clone();
        let mut shown = self.bitmap.frame.clone();
        for row in first..=last {
            hand_over(&mut cleared, &mut shown, row);
        }
        let page_value = self.languages.page_value();
        // An empty page writes nothing, so the marker is counted here.
        let others = cleared.stream(page_value).len().max(1);
        let mut left = LOCAL_TRIPLETS.saturating_sub(others);
        // A bitmap taking its rows now starts from rows of spaces, as the
        // page will show them; one that has them starts from its frame.
        let before = if first_present { shown } else { self.bitmap.frame.clone() };
        let colour_map = self.colour_map();
        let level = level_of(self.level);
        for attempt in 1..=PRESENT_ATTEMPTS {
            let mut frame = before.clone();
            let input = BitmapInput {
                sixels: &self.bitmap.back,
                text: &self.bitmap.text,
                first_row: usize::from(first),
                last_row: usize::from(last),
                level,
                colour_map: &colour_map,
                triplets: left,
            };
            // The host checked every colour and code as it was drawn, so
            // the engine refuses nothing; if it ever does, the script is
            // told and nothing has changed.
            let presented: Presented = present(&input, &mut frame).map_err(|error| range("BITMAP", &error.to_string()))?;
            let mut next = cleared.clone();
            show(&mut next, &frame, first, last);
            let used = next.stream(page_value).len();
            if used > LOCAL_TRIPLETS {
                let over = used.saturating_sub(LOCAL_TRIPLETS);
                left = if attempt.saturating_add(1) >= PRESENT_ATTEMPTS { 0 } else { left.saturating_sub(over) };
                continue;
            }
            self.change(|model| {
                *model = *next;
                Ok(())
            })?;
            self.bitmap.frame = frame;
            self.bitmap.rows = Some((first, last));
            self.bitmap.lossy = presented.lossy;
            let cells = i32::try_from(presented.cells).unwrap_or(0);
            let lossy = i32::try_from(presented.lossy).unwrap_or(0);
            return Ok(Reply::Int(lossy.saturating_mul(PRESENT_LOSSY_FACTOR).saturating_add(cells)));
        }
        Err(range(
            "LOCAL_TRIPLETS",
            &format!(
                "the bitmap's rows cannot be shown within the page's {LOCAL_TRIPLETS} triplets even by Level 1 codes alone: the overlay over them would need more; the page holds {} now",
                self.used
            ),
        ))
    }

    /// One operation of `nt.teletext.bitmap` (operations 130 to 138).
    pub(super) fn bitmap_operate(&mut self, op: Op, ints: &[i32], payload: Payload<'_>) -> Result<Reply, Reply> {
        match op {
            Op::BM_ROWS => {
                let [first, last] = exactly(op, ints)?;
                no_payload(op, payload)?;
                self.bitmap_rows(first, last)?;
            }
            Op::BM_CLEAR => {
                let [colour] = exactly(op, ints)?;
                no_payload(op, payload)?;
                let colour = colour_of(colour, "a sixel's colour")?;
                self.bitmap.back = [colour; BITMAP_SIXELS];
                self.bitmap.text = [BitmapText::NONE; BITMAP_CELLS];
            }
            Op::BM_SET => {
                let [x, y, colour] = exactly(op, ints)?;
                no_payload(op, payload)?;
                let colour = colour_of(colour, "a sixel's colour")?;
                self.bitmap.plot(x, y, colour);
            }
            Op::BM_GET => {
                let [x, y] = exactly(op, ints)?;
                no_payload(op, payload)?;
                // Outside the bitmap there is no sixel: undefined.
                return Ok(self.bitmap.sixel_mut(x, y).map_or(Reply::Undefined, |sixel| Reply::Int(i32::from(*sixel))));
            }
            Op::BM_LINE => {
                let [x0, y0, x1, y1, colour] = exactly(op, ints)?;
                no_payload(op, payload)?;
                let from = (coordinate_of(x0, "x0")?, coordinate_of(y0, "y0")?);
                let to = (coordinate_of(x1, "x1")?, coordinate_of(y1, "y1")?);
                let colour = colour_of(colour, "a sixel's colour")?;
                self.bitmap.line(from, to, colour);
            }
            Op::BM_RECT => {
                let [x, y, width, height, colour, fill] = exactly(op, ints)?;
                no_payload(op, payload)?;
                let (left, top) = (coordinate_of(x, "x")?, coordinate_of(y, "y")?);
                let (width, height) = (extent_of(width, "a width")?, extent_of(height, "a height")?);
                let colour = colour_of(colour, "a sixel's colour")?;
                let (right, bottom) = (left.saturating_add(width), top.saturating_add(height));
                match fill {
                    1 => self.bitmap.fill(left, top, right, bottom, colour),
                    // An outline: the four sides, each one sixel thick.
                    0 => {
                        self.bitmap.fill(left, top, right, top.saturating_add(1).min(bottom), colour);
                        self.bitmap.fill(left, bottom.saturating_sub(1).max(top), right, bottom, colour);
                        self.bitmap.fill(left, top, left.saturating_add(1).min(right), bottom, colour);
                        self.bitmap.fill(right.saturating_sub(1).max(left), top, right, bottom, colour);
                    }
                    _ => return Err(type_error("fill is true or false")),
                }
            }
            Op::BM_BLIT => self.bitmap_blit(op, ints, payload)?,
            Op::BM_TEXT => self.bitmap_text(op, ints, payload)?,
            Op::BM_PRESENT => {
                let [] = exactly(op, ints)?;
                no_payload(op, payload)?;
                return self.bitmap_present();
            }
            _ => return Err(Reply::Type(format!("operation {op:?} is not the bitmap's"))),
        }
        Ok(Reply::Undefined)
    }
}

#[cfg(test)]
mod tests;
