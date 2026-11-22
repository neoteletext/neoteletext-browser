//! A painter program as a drawing's contents or a macrograph's body
//! (script-api.md sections 6.3 and 10.2; work packages R1 and R2): the
//! whole numbers a script's `RegisPainter` calls became, checked one by
//! one and written as ReGIS by the host's own writer, so that what is
//! written is always one well-formed definition. The first fault refuses
//! the whole program. Each call is one of section 6.3's table and writes
//! ReGIS's own command for it: nothing ReGIS has is hidden and nothing it
//! lacks is made up.
//!
//! **Where the position is.** A drawing's contents start in a fresh
//! drawing context with [0,0] at the drawing's own origin (`regis-v2.md`,
//! "Rules for contents", rule 1). In ReGIS a signed number in brackets is
//! relative, so a place with a negative coordinate can only be written as
//! a move from where the position is, and the encoder always knows where
//! that is. Moves and vectors to a place say it themselves. For what
//! only the renderer's own arithmetic can say (where an arc about a
//! centre ends; how far a string advances with the text controls in
//! force; where a macrograph's body leaves it) the encoder asks the
//! renderer: a [`Pen`] is the renderer's own interpreter, given the same
//! bytes from the same place, and read for where it ended (TODO-7389).
//! Nothing of that arithmetic is written a second time here. A fill
//! puts the position back where it began, as ReGIS does. Only a
//! macrograph's body, which is drawn wherever it is called, has a
//! position nobody can know when it is defined: there a place with a
//! negative coordinate can be reached by a `moveTo` alone, which is
//! written from the origin, as it could only be in ReGIS itself.
//!
//! **Text** is written by the engine's own picture text encoder
//! (`neoteletext_formats::picture_text`, renderer parity PAR-3 and PAR-4),
//! the path a served picture's text takes: in NeoTeletext's 55 provided
//! character sets, alphabets 100 to 154, and no other (the owner's
//! DEC-6669 item 4), the language's own set first, right-to-left text in
//! visual order, and the plain Arabic-script letters in their base forms,
//! which the renderer joins. The fonts are the receiver's: nothing here
//! holds a glyph.

use std::io::Write as _;

use neoteletext_formats::picture_text::{self, PictureTextError};
use regis::{Screen, Terminal};

use super::super::{Reply, range, type_error};
use crate::limits::MAX_LANGUAGE_BYTES;
use crate::limits::regis::{
    ITALIC_ANGLES, MAX_ARC_DEGREES, MAX_CELL_SIDE, MAX_COORDINATE, MAX_FILL_VERTICES, MAX_PATTERN_BITS, MAX_PATTERN_MULTIPLIER, MAX_PLANES, MAX_POINTS,
    MAX_STANDARD_PATTERN, MAX_TEXT_CHARS, MAX_TEXT_HEIGHT, MAX_TEXT_SIZE, MAX_TILT, MIN_COORDINATE, OPTION_NOT_GIVEN, PIXEL_VECTOR_DIRECTIONS, REGIS_COLOURS,
    REGIS_MAX_CONTENTS_BYTES, TILT_STEP,
};
use crate::ops::RegisPaint;

/// The width of a character's display cell at the standard sizes, for
/// each unit of the size (the manual's chapter 7, table 7-3: size 0 and
/// size 1 are 9 pixels wide, size n is 9n).
const STANDARD_CELL_WIDTH: i32 = 9;
/// The height of a display cell at standard size 0, at size 1, and for
/// each unit of a larger size (table 7-3: 10, 20, and 15n).
const SIZE_0_HEIGHT: i32 = 10;
const SIZE_1_HEIGHT: i32 = 20;
const STANDARD_CELL_HEIGHT: i32 = 15;
/// The height a height multiplier counts in: the stored cell's ten pixels
/// (chapter 7, "Height Multiplier").
const HEIGHT_UNIT: i32 = 10;
/// The writing modes, in the order `mode` numbers them: overlay, replace,
/// complement, erase (chapter 3, "Writing Styles").
const MODES: [u8; 4] = *b"VRCE";
/// What `shade` asks for, as the program numbers it: off, on, to a
/// horizontal line at a y, to a vertical line at an x, with a character.
const SHADE_OFF: i32 = 0;
const SHADE_ON: i32 = 1;
const SHADE_TO_Y: i32 = 2;
const SHADE_TO_X: i32 = 3;
const SHADE_CHARACTER: i32 = 4;
/// What `pattern` gives, as the program numbers it: a standard pattern's
/// number, or eight bits.
const PATTERN_STANDARD: i32 = 0;
const PATTERN_BITS: i32 = 1;
/// The codes a shading character may be: the printable characters of the
/// built-in alphabet (2/1 to 7/14).
const FIRST_SHADING_CODE: i32 = 0x21;
const LAST_SHADING_CODE: i32 = 0x7E;
/// The end of a macrograph's definition (the manual's chapter 9), which
/// no body may hold.
const DEFINITION_END: &[u8] = b"@;";

/// A painter program being read: its whole numbers, front first.
pub(in crate::host) struct Words<'a>(pub(in crate::host) &'a [i32]);

impl<'a> Words<'a> {
    /// The next whole number.
    pub(in crate::host) fn next(&mut self) -> Result<i32, Reply> {
        let (first, rest) = self.0.split_first().ok_or_else(|| type_error("the painter program ends inside a call"))?;
        self.0 = rest;
        Ok(*first)
    }

    /// The next `count` whole numbers, as a program of their own.
    pub(in crate::host) fn take(&mut self, count: usize) -> Result<Words<'a>, Reply> {
        let (taken, rest) = self.0.split_at_checked(count).ok_or_else(|| type_error("the painter program ends inside a call"))?;
        self.0 = rest;
        Ok(Words(taken))
    }

    /// Whether nothing is left.
    pub(in crate::host) const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// A string: its length, then its code points, at most `most`.
    fn string(&mut self, most: usize, bound: &str) -> Result<String, Reply> {
        let length = usize::try_from(self.next()?).map_err(|_| type_error("a string's length is not negative"))?;
        if length > most {
            return Err(Reply::Range(format!("{bound}: a string of {length} characters is too long")));
        }
        self.take(length)?
            .0
            .iter()
            .map(|point| u32::try_from(*point).ok().and_then(char::from_u32).ok_or_else(|| type_error("a string's code points are Unicode scalar values")))
            .collect()
    }
}

/// `value` as a coordinate inside a drawing.
pub(in crate::host) fn coordinate(value: i32) -> Result<i32, Reply> {
    if (MIN_COORDINATE..=MAX_COORDINATE).contains(&value) {
        Ok(value)
    } else {
        Err(range("MAX_COORDINATE", "a coordinate inside a drawing is -32,768 to 32,767"))
    }
}

/// The move a pixel vector's direction makes, in pixels: 0 is east and
/// the directions go anticlockwise, y downwards (chapter 4, "Pixel
/// Vector").
const fn direction(number: i32) -> (i32, i32) {
    match number.rem_euclid(PIXEL_VECTOR_DIRECTIONS) {
        0 => (1, 0),
        1 => (1, -1),
        2 => (0, -1),
        3 => (-1, -1),
        4 => (-1, 0),
        5 => (-1, 1),
        6 => (0, 1),
        _ => (1, 1),
    }
}

/// What a program is written for, and what it may name.
pub(in crate::host) struct Setting<'a> {
    /// The language text is written in when a call names none: the
    /// context's, a tag of the engine's table of languages.
    pub(in crate::host) language: &'a str,
    /// The letter of the macrograph a handle names, for the context that
    /// gave the program; the error for a handle that names none.
    pub(in crate::host) letter: &'a dyn Fn(i32) -> Result<u8, Reply>,
    /// Whether the program is a macrograph's body, which runs wherever it
    /// is called, and not a drawing's contents.
    pub(in crate::host) macrograph: bool,
}

/// The renderer's own interpreter, asked where the position is after
/// bytes the encoder wrote: a terminal that is fed them from the place
/// the encoder says and read for the place it ended at. It holds the
/// page's macrographs as the host defined them, and the text controls of
/// the drawing being written, since both decide where a call or a string
/// leaves the position.
pub(in crate::host) struct Pen {
    terminal: Terminal,
    screen: Screen,
}

/// What a fresh drawing context's text controls and pixel-vector
/// multiplier are (the manual's chapter 1, table 1-2), given to the pen
/// before each drawing's contents are written: alphabet 0, standard size
/// 1, no tilt, no slant, a multiplier of 1.
const FRESH_CONTEXT: &[u8] = b"T(A0,S1,D0,I0)W(M1)";
/// The bytes a pen's screen holds: two pages of 800 by 480 pixels.
const PEN_SCREEN_BYTES: usize = 2 * (regis::SCREEN_WIDTH as usize) * (regis::SCREEN_HEIGHT as usize);
/// Two places a macrograph is called from to learn how it leaves the
/// position: far enough apart that a body which goes to a place of its
/// own and one which moves from where it was called are told apart.
const LEAVE_FROM: [(i32, i32); 2] = [(1000, 1000), (3000, 2000)];
/// A character drawn after the call, to learn the advance the body left
/// the text controls with.
const LEAVE_TEXT: &[u8] = b"T'x'";

impl Pen {
    pub(in crate::host) fn new() -> Self {
        Self { terminal: Terminal::new(), screen: Screen::new() }
    }

    /// The bytes held.
    pub(in crate::host) fn held_bytes(&self) -> usize {
        self.terminal.held_bytes().saturating_add(PEN_SCREEN_BYTES)
    }

    fn feed(&mut self, bytes: &[u8]) {
        let _ = self.terminal.feed_regis(&mut self.screen, bytes);
    }

    /// Begins a drawing's contents: a fresh drawing context.
    fn begin(&mut self) {
        self.feed(FRESH_CONTEXT);
    }

    /// Where the position is after `bytes`, written from the place `from`.
    fn after(&mut self, from: (i32, i32), bytes: &[u8]) -> (i32, i32) {
        // To the place: as it stands, or from the origin when a
        // coordinate is negative.
        let mut to = Vec::new();
        let _ = if from.0 >= 0 && from.1 >= 0 { write!(to, "P[{},{}]", from.0, from.1) } else { write!(to, "P[0,0]P[{:+},{:+}]", from.0, from.1) };
        self.feed(&to);
        self.feed(bytes);
        self.terminal.position()
    }

    /// Defines the macrograph `letter` as the host has it.
    pub(in crate::host) fn define(&mut self, letter: u8, body: &[u8]) {
        let mut text = vec![b'@', b':', letter];
        text.extend_from_slice(body);
        text.extend_from_slice(b"@;");
        self.feed(&text);
    }

    /// How the macrograph `letter` leaves the position and the text's
    /// advance, as far as calling it shows: where it ends, and where a
    /// character drawn after it ends, called from each of two places in
    /// a fresh context.
    pub(in crate::host) fn leave(&mut self, letter: u8) -> [(i32, i32); 4] {
        let mut ends = [(0, 0); 4];
        for (from, pair) in LEAVE_FROM.iter().zip(ends.as_chunks_mut::<2>().0) {
            self.begin();
            let after_call = self.after(*from, &[b'@', letter]);
            self.feed(LEAVE_TEXT);
            *pair = [after_call, self.terminal.position()];
        }
        ends
    }
}

/// Writes a drawing's contents from a painter program.
pub(in crate::host) struct Encoder<'a> {
    setting: &'a Setting<'a>,
    pen: &'a mut Pen,
    /// The macrographs the contents call: bit 0 for A.
    calls: u32,
    /// The contents so far.
    out: Vec<u8>,
    /// Where the position is, from the drawing's origin, while it is known.
    at: Option<(i32, i32)>,
    /// Whether the calls being read are inside a `fill`.
    filling: bool,
    /// The vertices the `fill` being read has traced.
    fill_vertices: usize,
}

/// The `TypeError` for a call in a macrograph's body that needs the
/// position, which a body does not have.
fn unknown_position() -> Reply {
    type_error("a macrograph is drawn wherever it is called, so its body has no position of its own: moveTo first to reach a negative coordinate")
}

/// The fault of the engine's picture text encoder as the API's error.
fn text_fault(fault: &PictureTextError, text: &str) -> Reply {
    match fault {
        PictureTextError::TooLong { .. } => range("MAX_TEXT_CHARS", "a string is at most 1,000 characters"),
        PictureTextError::Unshown { index } | PictureTextError::Forbidden { index } => {
            let character = text.chars().nth(*index).unwrap_or(char::REPLACEMENT_CHARACTER);
            Reply::Type(format!("no provided character set of the language shows the character U+{:04X} of the string, at {index}", u32::from(character)))
        }
        PictureTextError::UnknownLanguage(tag) => Reply::Type(format!("'{tag}' is not a language the provided character sets write")),
        _ => type_error("the string cannot be written as ReGIS text"),
    }
}

/// The options of one `text` call or of `measure`, each given or not.
#[derive(Clone, Copy, Debug)]
pub(in crate::host) struct TextOptions {
    size: Option<i32>,
    cell: Option<(i32, i32)>,
    unit: Option<(i32, i32)>,
    height: Option<i32>,
    tilt: Option<i32>,
    italic: Option<i32>,
    spacing: Option<(i32, i32)>,
}

/// An option's whole number, or none when it was not given.
const fn given(value: i32) -> Option<i32> {
    if value == OPTION_NOT_GIVEN { None } else { Some(value) }
}

/// Two whole numbers as a pair an option takes whole or not at all.
fn pair(first: i32, second: i32, what: &str) -> Result<Option<(i32, i32)>, Reply> {
    match (given(first), given(second)) {
        (Some(first), Some(second)) => Ok(Some((first, second))),
        (None, None) => Ok(None),
        _ => Err(Reply::Type(format!("{what} is two whole numbers"))),
    }
}

impl TextOptions {
    /// The options from their whole numbers, each held to ReGIS's range
    /// (the manual's chapter 7): the standard size, the display cell, the
    /// unit cell, the height multiplier, the tilt, the italic slant and
    /// the spacing.
    pub(in crate::host) fn read(numbers: [i32; 10]) -> Result<Self, Reply> {
        let [size, cell_width, cell_height, unit_width, unit_height, height, tilt, italic, spacing_x, spacing_y] = numbers;
        let options = Self {
            size: given(size),
            cell: pair(cell_width, cell_height, "a text cell")?,
            unit: pair(unit_width, unit_height, "a text unit")?,
            height: given(height),
            tilt: given(tilt),
            italic: given(italic),
            spacing: pair(spacing_x, spacing_y, "a text spacing")?,
        };
        if options.size.is_some_and(|size| !(0..=MAX_TEXT_SIZE).contains(&size)) {
            return Err(range("MAX_TEXT_SIZE", "a text size is 0 to 16"));
        }
        for (width, height) in options.cell.into_iter().chain(options.unit) {
            if !(1..=MAX_CELL_SIDE).contains(&width) || !(1..=MAX_CELL_SIDE).contains(&height) {
                return Err(range("MAX_CELL_SIDE", "a text cell's sides are 1 to 2,560 pixels"));
            }
        }
        if options.height.is_some_and(|height| !(1..=MAX_TEXT_HEIGHT).contains(&height)) {
            return Err(range("MAX_TEXT_HEIGHT", "a text height is 1 to 256"));
        }
        if options.tilt.is_some_and(|tilt| !(-MAX_TILT..=MAX_TILT).contains(&tilt) || tilt.checked_rem(TILT_STEP) != Some(0)) {
            return Err(type_error("a text tilt is a multiple of 45 degrees, -360 to 360"));
        }
        if options.italic.is_some_and(|italic| !ITALIC_ANGLES.contains(&italic.saturating_abs())) {
            return Err(type_error("a text italic is 0, 22 or 45 degrees, either way"));
        }
        if let Some((x, y)) = options.spacing {
            coordinate(x)?;
            coordinate(y)?;
        }
        Ok(options)
    }

    /// The display cell these options alone select, from the standard
    /// size 1 a drawing starts with: its width and height in pixels.
    pub(in crate::host) fn cell(&self) -> (i32, i32) {
        let (mut width, mut height) = (STANDARD_CELL_WIDTH, SIZE_1_HEIGHT);
        if let Some(size) = self.size {
            width = STANDARD_CELL_WIDTH.saturating_mul(size.max(1));
            height = match size {
                0 => SIZE_0_HEIGHT,
                1 => SIZE_1_HEIGHT,
                _ => STANDARD_CELL_HEIGHT.saturating_mul(size),
            };
        }
        if let Some(cell) = self.cell {
            (width, height) = cell;
        }
        if let Some(multiplier) = self.height {
            height = HEIGHT_UNIT.saturating_mul(multiplier);
        }
        (width, height)
    }

    /// The tilt as a pixel vector's direction, when a tilt was given.
    pub(in crate::host) fn direction(&self) -> Option<i32> {
        self.tilt.and_then(|tilt| tilt.checked_div(TILT_STEP)).map(|steps| steps.rem_euclid(PIXEL_VECTOR_DIRECTIONS))
    }
}

/// `text` in `language` as the engine's picture text encoder writes it:
/// its ReGIS text commands and the character cells it advances.
///
/// `named` is the language a call names, which must be one the engine
/// writes; with none the text is in `own`, the context's language, or in
/// Latin English when the engine has no set for that, as a page's is.
pub(in crate::host) fn written_text(text: &str, named: &str, own: &str) -> Result<(Vec<u8>, i32), Reply> {
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(range("MAX_TEXT_CHARS", "a string is at most 1,000 characters"));
    }
    let known = |tag: &str| neoteletext_core::charset::language::language(tag).is_some();
    let language = if named.is_empty() {
        known(own).then_some(own)
    } else if known(named) {
        Some(named)
    } else {
        return Err(Reply::Type(format!("'{named}' is not a language the provided character sets write")));
    };
    let stretches = picture_text::stretches(text, language, false).map_err(|fault| text_fault(&fault, text))?;
    let mut bytes = Vec::new();
    picture_text::regis(&stretches, &mut bytes).map_err(|fault| text_fault(&fault, text))?;
    let cells = i32::try_from(picture_text::measure(&stretches).cells).unwrap_or(i32::MAX);
    Ok((bytes, cells))
}

impl<'a> Encoder<'a> {
    /// An encoder at the start of a drawing's contents or a macrograph's body.
    fn new(setting: &'a Setting<'a>, pen: &'a mut Pen) -> Self {
        pen.begin();
        Self { setting, pen, calls: 0, out: Vec::new(), at: (!setting.macrograph).then_some((0, 0)), filling: false, fill_vertices: 0 }
    }

    /// Takes the position to where the renderer's own arithmetic puts
    /// it after the bytes written since `start`: asked of the pen, from
    /// the place the position was. A macrograph's body has no place to
    /// ask from.
    fn ask(&mut self, start: usize) -> Result<(), Reply> {
        let Some(from) = self.at else { return Ok(()) };
        let (x, y) = self.pen.after(from, self.out.get(start..).unwrap_or_default());
        self.at = Some((coordinate(x)?, coordinate(y)?));
        Ok(())
    }

    /// Writes text; writing to a vector cannot fail.
    fn put(&mut self, text: core::fmt::Arguments<'_>) {
        let _ = self.out.write_fmt(text);
    }

    /// The place `[x, y]` in brackets, as ReGIS takes it: as it stands
    /// when neither coordinate is negative, otherwise as a move from where
    /// the position is, since a signed number in brackets is relative (the
    /// manual's chapter 1, "Relative Positioning").
    fn bracket(&mut self, x: i32, y: i32) -> Result<(), Reply> {
        if x >= 0 && y >= 0 {
            self.put(format_args!("[{x},{y}]"));
        } else {
            let (from_x, from_y) = self.at.ok_or_else(unknown_position)?;
            self.put(format_args!("[{:+},{:+}]", x.saturating_sub(from_x), y.saturating_sub(from_y)));
        }
        Ok(())
    }

    /// Writes the place `[x, y]` and takes the position there.
    fn place(&mut self, x: i32, y: i32) -> Result<(), Reply> {
        self.bracket(x, y)?;
        self.at = Some((x, y));
        Ok(())
    }

    /// Moves to `[x, y]`: from the origin when the place has a negative
    /// coordinate and the position is not known.
    fn move_to(&mut self, x: i32, y: i32) -> Result<(), Reply> {
        self.out.push(b'P');
        if (x < 0 || y < 0) && self.at.is_none() {
            self.out.extend_from_slice(b"[0,0]P");
            self.at = Some((0, 0));
        }
        self.place(x, y)
    }

    /// Takes the position on by `(dx, dy)`, while it is known.
    fn moved_by(&mut self, dx: i32, dy: i32) -> Result<(), Reply> {
        if let Some((x, y)) = self.at {
            self.at = Some((coordinate(x.saturating_add(dx))?, coordinate(y.saturating_add(dy))?));
        }
        Ok(())
    }

    /// Counts vertices of the outline a `fill` traces.
    fn vertices(&mut self, count: usize) -> Result<(), Reply> {
        if self.filling {
            self.fill_vertices = self.fill_vertices.saturating_add(count);
            if self.fill_vertices > MAX_FILL_VERTICES {
                return Err(range("MAX_FILL_VERTICES", "a fill traces at most 4,096 vertices"));
            }
        }
        Ok(())
    }

    /// Refuses a write control inside a `fill`, where only the outline
    /// matters (chapter 11, "Temporary Write Controls").
    fn outside_fill(&self, what: &str) -> Result<(), Reply> {
        if self.filling { Err(Reply::Type(format!("{what} is chosen before a fill, not inside it"))) } else { Ok(()) }
    }

    /// The points of a `polyline`, `polygon` or `curve`: their count,
    /// then each.
    fn points(words: &mut Words<'_>) -> Result<Vec<(i32, i32)>, Reply> {
        let count = usize::try_from(words.next()?).ok().filter(|count| *count >= 1).ok_or_else(|| type_error("a list of points has at least one point"))?;
        if count > MAX_POINTS {
            return Err(range("MAX_POINTS", "a list of points has at most 1,024 points"));
        }
        (0..count).map(|_| Ok((coordinate(words.next()?)?, coordinate(words.next()?)?))).collect()
    }

    /// `text(string, options)`: the options as one text command, then the
    /// string as the engine's picture text encoder writes it.
    fn text(&mut self, words: &mut Words<'_>) -> Result<(), Reply> {
        let mut numbers = [0_i32; 10];
        for number in &mut numbers {
            *number = words.next()?;
        }
        let options = TextOptions::read(numbers)?;
        let language = words.string(MAX_LANGUAGE_BYTES, "MAX_LANGUAGE_BYTES")?;
        let string = words.string(MAX_TEXT_CHARS, "MAX_TEXT_CHARS")?;
        self.outside_fill("text")?;
        let (written, _) = written_text(&string, &language, self.setting.language)?;
        let start = self.out.len();
        // The options, in the order that lets each stand: a standard size
        // sets both cells and forgets the spacing, so it comes first.
        let mut parts: Vec<String> = Vec::new();
        if let Some(size) = options.size {
            parts.push(format!("S{size}"));
        }
        if let Some((width, height)) = options.cell {
            parts.push(format!("S[{width},{height}]"));
        }
        if let Some((width, height)) = options.unit {
            parts.push(format!("U[{width},{height}]"));
        }
        if let Some(height) = options.height {
            parts.push(format!("H{height}"));
        }
        if let Some(tilt) = options.tilt {
            parts.push(format!("D{tilt}"));
        }
        if let Some(italic) = options.italic {
            parts.push(format!("I{italic}"));
        }
        if !parts.is_empty() || options.spacing.is_some() {
            self.out.push(b'T');
            if !parts.is_empty() {
                self.put(format_args!("({})", parts.join(",")));
            }
            if let Some((x, y)) = options.spacing {
                self.put(format_args!("[{x:+},{y:+}]"));
            }
        }
        self.out.extend_from_slice(&written);
        // Where the string leaves the position is the renderer's to say:
        // its advance with the text controls now in force.
        self.ask(start)
    }

    /// Reads and writes one call.
    #[expect(clippy::too_many_lines, reason = "one arm for each call of section 6.3's table")]
    fn call(&mut self, words: &mut Words<'_>) -> Result<(), Reply> {
        let number = words.next()?;
        let call = RegisPaint::from_number(number).ok_or_else(|| type_error("the painter program holds a call no painter has"))?;
        match call {
            RegisPaint::MOVE_TO => {
                let (x, y) = (coordinate(words.next()?)?, coordinate(words.next()?)?);
                self.move_to(x, y)?;
            }
            RegisPaint::MOVE_BY | RegisPaint::LINE_BY => {
                let (dx, dy) = (words.next()?, words.next()?);
                if !(MIN_COORDINATE.saturating_mul(2)..=MAX_COORDINATE.saturating_mul(2)).contains(&dx)
                    || !(MIN_COORDINATE.saturating_mul(2)..=MAX_COORDINATE.saturating_mul(2)).contains(&dy)
                {
                    return Err(range("MAX_COORDINATE", "a move is no longer than the coordinates' whole range"));
                }
                self.moved_by(dx, dy)?;
                if call == RegisPaint::LINE_BY {
                    self.vertices(1)?;
                }
                self.out.push(if call == RegisPaint::MOVE_BY { b'P' } else { b'V' });
                self.put(format_args!("[{dx:+},{dy:+}]"));
            }
            RegisPaint::LINE_TO => {
                let (x, y) = (coordinate(words.next()?)?, coordinate(words.next()?)?);
                self.vertices(1)?;
                self.out.push(b'V');
                self.place(x, y)?;
            }
            RegisPaint::DOT => {
                self.vertices(1)?;
                self.out.extend_from_slice(b"V[]");
            }
            RegisPaint::POLYLINE | RegisPaint::POLYGON => {
                let points = Self::points(words)?;
                self.vertices(points.len())?;
                let mut points = points.into_iter();
                let Some((first_x, first_y)) = points.next() else { return Ok(()) };
                self.move_to(first_x, first_y)?;
                // A bounded sequence closes back to its first point
                // (chapter 4, "Bounded Sequence"); an open one ends where
                // its last vector ends. One point alone is a dot.
                self.out.extend_from_slice(if call == RegisPaint::POLYGON { b"V(B)" } else { b"V" });
                let mut drawn = false;
                for (x, y) in points {
                    self.place(x, y)?;
                    drawn = true;
                }
                if !drawn {
                    self.out.extend_from_slice(b"[]");
                }
                if call == RegisPaint::POLYGON {
                    self.out.extend_from_slice(b"(E)");
                    self.at = Some((first_x, first_y));
                }
            }
            RegisPaint::RECT => {
                let (x, y) = (coordinate(words.next()?)?, coordinate(words.next()?)?);
                let (width, height) = (words.next()?, words.next()?);
                if width < 1 || height < 1 {
                    return Err(type_error("a rectangle's width and height are at least 1"));
                }
                // A rectangle of `width` pixels reaches from its left
                // column to the one `width - 1` further on.
                let (across, down) = (width.saturating_sub(1), height.saturating_sub(1));
                coordinate(x.saturating_add(across))?;
                coordinate(y.saturating_add(down))?;
                self.vertices(4)?;
                self.move_to(x, y)?;
                self.put(format_args!("V[+{across}][,+{down}][-{across}][,-{down}]"));
            }
            RegisPaint::VECTOR => {
                // Pixel vectors: each digit a move of `multiplier` pixels
                // in one of eight directions. The multiplier is given as
                // a write control of this command alone, so that it
                // changes nothing after it (chapter 3, "PV Multiplication").
                let multiplier = words.next()?;
                if !(1..=MAX_COORDINATE).contains(&multiplier) {
                    return Err(range("MAX_COORDINATE", "a pixel vector's multiplier is 1 to 32,767"));
                }
                let count =
                    usize::try_from(words.next()?).ok().filter(|count| *count >= 1).ok_or_else(|| type_error("a pixel vector has at least one digit"))?;
                if count > MAX_POINTS {
                    return Err(range("MAX_POINTS", "a pixel vector has at most 1,024 digits"));
                }
                let digits = words.take(count)?;
                if digits.0.iter().any(|digit| !(0..PIXEL_VECTOR_DIRECTIONS).contains(digit)) {
                    return Err(type_error("a pixel vector's digits are 0 to 7"));
                }
                self.vertices(count)?;
                for digit in digits.0 {
                    let (x, y) = direction(*digit);
                    self.moved_by(x.saturating_mul(multiplier), y.saturating_mul(multiplier))?;
                }
                self.put(format_args!("V(W(M{multiplier}))"));
                for digit in digits.0 {
                    self.put(format_args!("{digit}"));
                }
            }
            RegisPaint::CIRCLE => {
                // A circle about the position, through the point `radius`
                // to its right; the position stays (chapter 6).
                let radius = words.next()?;
                if !(0..=MAX_COORDINATE).contains(&radius) {
                    return Err(range("MAX_COORDINATE", "a radius is 0 to 32,767"));
                }
                self.put(format_args!("C[+{radius}]"));
            }
            RegisPaint::CIRCLE_THROUGH => {
                let (x, y) = (coordinate(words.next()?)?, coordinate(words.next()?)?);
                self.out.push(b'C');
                self.bracket(x, y)?;
            }
            RegisPaint::ARC => {
                // An arc from the position about a centre: `C(C,A d)`.
                // It leaves the position at its end, so that arcs link;
                // where that is, the renderer's own arithmetic says.
                let (x, y) = (coordinate(words.next()?)?, coordinate(words.next()?)?);
                let degrees = words.next()?;
                if !(-MAX_ARC_DEGREES..=MAX_ARC_DEGREES).contains(&degrees) || degrees == 0 {
                    return Err(type_error("an arc is -360 to 360 degrees, and not none"));
                }
                let start = self.out.len();
                self.put(format_args!("C(C,A{degrees:+})"));
                self.bracket(x, y)?;
                self.ask(start)?;
            }
            RegisPaint::CURVE => {
                // ReGIS's interpolated curve through the points, from the
                // first: open, `C(S)…(E)`, ending at the last, or closed,
                // `C(B)…(E)`, ending where it began (chapter 6).
                let closed = match words.next()? {
                    0 => false,
                    1 => true,
                    _ => return Err(type_error("a curve is open (0) or closed (1)")),
                };
                let points = Self::points(words)?;
                let mut points = points.into_iter();
                let Some((first_x, first_y)) = points.next() else { return Ok(()) };
                self.move_to(first_x, first_y)?;
                self.out.extend_from_slice(if closed { b"C(B)" } else { b"C(S)" });
                for (x, y) in points {
                    self.place(x, y)?;
                }
                self.out.extend_from_slice(b"(E)");
                if closed {
                    self.at = Some((first_x, first_y));
                }
            }
            RegisPaint::FILL => {
                if self.filling {
                    return Err(type_error("a fill is not traced inside another fill"));
                }
                let length = usize::try_from(words.next()?).map_err(|_| type_error("a fill's length is not negative"))?;
                let mut body = words.take(length)?;
                self.filling = true;
                self.fill_vertices = 0;
                // A fill puts the position back where it began, "whether
                // or not any drawing takes place" (chapter 11).
                let began = self.at;
                self.out.extend_from_slice(b"F(");
                while !body.is_empty() {
                    self.call(&mut body)?;
                }
                self.out.push(b')');
                self.filling = false;
                self.at = began;
            }
            RegisPaint::INK => {
                let entry = words.next()?;
                self.outside_fill("the ink")?;
                if usize::try_from(entry).map_or(true, |entry| entry >= REGIS_COLOURS) {
                    return Err(range("REGIS_COLOURS", "an ink is a colour-map entry, 0 to 15"));
                }
                self.put(format_args!("W(I{entry})"));
            }
            RegisPaint::MODE => {
                let mode = words.next()?;
                self.outside_fill("the writing mode")?;
                let letter = usize::try_from(mode)
                    .ok()
                    .and_then(|mode| MODES.get(mode))
                    .ok_or_else(|| type_error("a writing mode is overlay, replace, complement or erase"))?;
                self.out.extend_from_slice(&[b'W', b'(', *letter, b')']);
            }
            RegisPaint::NEGATIVE => {
                let on = words.next()?;
                self.outside_fill("negative writing")?;
                if !(0..=1).contains(&on) {
                    return Err(type_error("negative writing is on (1) or off (0)"));
                }
                self.put(format_args!("W(N{on})"));
            }
            RegisPaint::PATTERN => {
                let (kind, value, multiplier) = (words.next()?, words.next()?, words.next()?);
                self.outside_fill("the pattern")?;
                if !(1..=MAX_PATTERN_MULTIPLIER).contains(&multiplier) {
                    return Err(range("MAX_PATTERN_MULTIPLIER", "a pattern's multiplier is 1 to 16"));
                }
                match kind {
                    PATTERN_STANDARD if (0..=MAX_STANDARD_PATTERN).contains(&value) => self.put(format_args!("W(P{value}(M{multiplier}))")),
                    // Eight binary digits are a pattern of their own
                    // (chapter 3, "Specified Binary Pattern").
                    PATTERN_BITS if (0..=MAX_PATTERN_BITS).contains(&value) => self.put(format_args!("W(P{value:08b}(M{multiplier}))")),
                    _ => return Err(type_error("a pattern is a standard pattern, 0 to 9, or eight bits")),
                }
            }
            RegisPaint::PLANES => {
                let mask = words.next()?;
                self.outside_fill("the planes")?;
                if !(0..=MAX_PLANES).contains(&mask) {
                    return Err(range("MAX_PLANES", "a plane mask is 0 to 15"));
                }
                self.put(format_args!("W(F{mask})"));
            }
            RegisPaint::SHADE => {
                let (kind, value) = (words.next()?, words.next()?);
                self.outside_fill("shading")?;
                match kind {
                    SHADE_OFF => self.out.extend_from_slice(b"W(S0)"),
                    SHADE_ON => self.out.extend_from_slice(b"W(S1)"),
                    // To a reference line through a place: a signed
                    // number is relative here too.
                    SHADE_TO_Y | SHADE_TO_X => {
                        let line = coordinate(value)?;
                        let from = match (line < 0, self.at) {
                            (false, _) => None,
                            (true, Some((x, y))) => Some(if kind == SHADE_TO_Y { y } else { x }),
                            (true, None) => return Err(unknown_position()),
                        };
                        let written = from.map_or_else(|| line.to_string(), |from| format!("{:+}", line.saturating_sub(from)));
                        if kind == SHADE_TO_Y {
                            self.put(format_args!("W(S[,{written}])"));
                        } else {
                            self.put(format_args!("W(S(X)[{written}])"));
                        }
                    }
                    SHADE_CHARACTER if (FIRST_SHADING_CODE..=LAST_SHADING_CODE).contains(&value) => {
                        let code = u8::try_from(value).unwrap_or(b'#');
                        // A quote is doubled inside a quoted string.
                        let doubled = if code == b'\'' { "'" } else { "" };
                        self.put(format_args!("W(S'{doubled}{}')", char::from(code)));
                    }
                    _ => return Err(type_error("shading is off, on, to a y, to an x, or with one printable ASCII character")),
                }
            }
            RegisPaint::TEXT => self.text(words)?,
            RegisPaint::CALL => {
                let handle = words.next()?;
                let letter = (self.setting.letter)(handle)?;
                let start = self.out.len();
                self.out.extend_from_slice(&[b'@', letter]);
                self.calls |= 1_u32.checked_shl(u32::from(letter.saturating_sub(b'A'))).unwrap_or(0);
                // Where the body leaves the position, and the controls it
                // sets, are the renderer's to say.
                self.ask(start)?;
            }
        }
        if self.out.len() > REGIS_MAX_CONTENTS_BYTES {
            return Err(range("REGIS_MAX_CONTENTS_BYTES", "a drawing's contents are at most 65,536 bytes"));
        }
        Ok(())
    }
}

/// What the painter program `words` writes, for `setting`, with the
/// macrographs it calls (bit 0 for A), or the error that refuses it
/// whole. `pen` is asked where the position is.
pub(in crate::host) fn encode(words: &[i32], setting: &Setting<'_>, pen: &mut Pen) -> Result<(Vec<u8>, u32), Reply> {
    let mut words = Words(words);
    let mut encoder = Encoder::new(setting, pen);
    while !words.is_empty() {
        encoder.call(&mut words)?;
    }
    // A macrograph's definition is stored up to the first `@;` whatever
    // surrounds it (chapter 9), so its body holds none: a call of
    // macrograph `;` does not exist, and text is in double quotes that
    // the definition does not read.
    if setting.macrograph && encoder.out.windows(DEFINITION_END.len()).any(|pair| pair == DEFINITION_END) {
        return Err(type_error("a macrograph's text does not hold '@;', which ends a macrograph's definition"));
    }
    Ok((encoder.out, encoder.calls))
}
