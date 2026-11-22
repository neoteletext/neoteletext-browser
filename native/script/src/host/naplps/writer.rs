//! The NAPLPS a `NaplpsPainter` writes (script-api.md section 7.3; ANSI
//! X3.110-1983, cited by section), in the 7-bit form.
//!
//! A [`Painter`] holds two things: the **pen**, which is what the script
//! last asked for (the drawing point, the ink, the logical pel, the line
//! and fill textures), and what was **sent**, which is what the receiver
//! holds after the bytes written so far (its drawing point, its colour
//! mode and entries, its DOMAIN, TEXTURE and TEXT, and whether the PDI
//! set is in use). Before a call draws, the differences between the two
//! are written and nothing else, so a script cannot desynchronise the
//! stream and no state is written twice.
//!
//! Places are dots of the 480 by 460 canvas, origin top left, y down
//! (DEC-6853, DEC-6857). A dot is written as the six-byte operand of
//! neoteletext-common's `docs/naplps-raster.md` RAS-2 and RAS-6, by the
//! renderer's own `dot_steps`, so every absolute place is exact. A
//! relative form's step is the difference between the operand of the
//! place it must reach and the operand of where the receiver's drawing
//! point is, so a chain of steps lands on its dots as exactly as
//! absolute places do, however long it is.

use naplps::limits::{DISPLAY_LINES, RASTER_WIDTH};
use naplps::{Rounding, dot_steps, multi_value_operand, unit_steps};
use neoteletext_formats::picture_text::{self, PictureTextError, Stretch};

use super::Fault;
use super::limits::{
    CANVAS_HEIGHT, CANVAS_TOP_LINE, CANVAS_WIDTH, CELL_DOTS, CELL_LINES, FIRST_TEXT_ROW, GRID_COLUMNS, LAST_TEXT_ROW, MAX_MASK_DOTS, MAX_POINTS,
    MAX_POLYGON_VERTICES, MAX_TEXT_CHARS, MAX_TRACE_MOVES, MAX_WAIT_TENTHS, MAX_X, MAX_Y, MIN_X, MIN_Y, NAPLPS_COLOURS, NAPLPS_MAX_IMAGE_HEIGHT,
    NAPLPS_MAX_IMAGE_PELS, PALETTE_BITS,
};
use super::program::{Paint, Words};

/// Shift out: the PDI set in use (section 4.3.2). The text encoder's own
/// bytes shift G0 back in.
const SO: u8 = 0x0E;
/// Shift in: G0 in use.
const SI: u8 = 0x0F;
/// Single shifts 2 and 3: the next character from G2 or G3.
const SS2: u8 = 0x19;
const SS3: u8 = 0x1D;
/// A C0 code that does nothing but end the PDI before it, so that it is
/// drawn (the renderer's `control0`).
const END_PDI: u8 = 0x1A;
const ESC: u8 = 0x1B;
/// The designations of a 94-character set into G2 and G3, and the final
/// bytes of the supplementary, macro and DRCS sets (section 4.3.2, Table 1).
const INTO_G2: u8 = 0x2A;
const INTO_G3: u8 = 0x2B;
const SUPPLEMENTARY_SET: u8 = 0x7C;
const MACRO_SET: u8 = 0x7A;
const DRCS_SET: u8 = 0x7B;
/// The C1 controls a painter writes, as the final byte after ESC (section
/// 6.2): reverse video on and off, underline on and off.
const REVERSE_ON: u8 = 0x48;
const REVERSE_OFF: u8 = 0x49;
const UNDERLINE_ON: u8 = 0x59;
const UNDERLINE_OFF: u8 = 0x5A;

/// The PDI opcodes (section 5.3.1.1, Figure 9).
mod opcode {
    pub const RESET: u8 = 0x20;
    pub const DOMAIN: u8 = 0x21;
    pub const TEXT: u8 = 0x22;
    pub const TEXTURE: u8 = 0x23;
    pub const POINT_SET_ABSOLUTE: u8 = 0x24;
    pub const POINT_SET_RELATIVE: u8 = 0x25;
    pub const POINT_ABSOLUTE: u8 = 0x26;
    pub const POINT_RELATIVE: u8 = 0x27;
    pub const LINE_ABSOLUTE: u8 = 0x28;
    pub const LINE_RELATIVE: u8 = 0x29;
    pub const SET_LINE_ABSOLUTE: u8 = 0x2A;
    pub const ARC_OUTLINED: u8 = 0x2C;
    pub const RECTANGLE_OUTLINED: u8 = 0x30;
    pub const SET_RECTANGLE_OUTLINED: u8 = 0x32;
    pub const POLYGON_OUTLINED: u8 = 0x34;
    pub const SET_POLYGON_OUTLINED: u8 = 0x36;
    pub const FIELD: u8 = 0x38;
    pub const INCREMENTAL_POINT: u8 = 0x39;
    pub const INCREMENTAL_LINE: u8 = 0x3A;
    pub const INCREMENTAL_POLYGON: u8 = 0x3B;
    pub const SET_COLOR: u8 = 0x3C;
    pub const WAIT: u8 = 0x3D;
    pub const SELECT_COLOR: u8 = 0x3E;
    pub const BLINK: u8 = 0x3F;
    /// What the filled form of an outlined shape's opcode adds to it.
    pub const FILLED: u8 = 1;
}

/// The bit every operand byte carries, and its six data bits (section 5.3.1).
const OPERAND: u8 = 0x40;
const OPERAND_DATA_BITS: u8 = 6;
const DATA_BITS: u32 = OPERAND_DATA_BITS as u32;
const DATA_MASK: u8 = 0x3F;
/// RESET with every bit (section 5.3.2.9, Tables 14 and 15): the domain,
/// the colour map, the screen cleared to black, text, blinks, textures,
/// macros and DRCS.
const RESET_EVERYTHING: [u8; 2] = [0x7F, 0x7F];
/// Multi-value operands of six bytes (RAS-6), and DOMAIN's fixed-format
/// byte for them: one-byte single values, two dimensions (section
/// 5.3.2.2.1, Figure 14).
const OPERAND_BYTES: u8 = 6;
const DOMAIN_FORMAT: u8 = OPERAND | ((OPERAND_BYTES - 1) << 2);
/// The largest and smallest value of one axis of a six-byte operand:
/// eighteen bits of two's complement.
const AXIS_MAX: i64 = (1 << (3 * OPERAND_BYTES as u32 - 1)) - 1;
const AXIS_MIN: i64 = -(1 << (3 * OPERAND_BYTES as u32 - 1));
/// WAIT's fixed-format byte (section 5.3.2.8.2).
const WAIT_FORMAT: u8 = 0x5C;
/// The share of the unit screen's height the canvas's lines span, 25/32
/// (the renderer's surface, after ANSI X3.110-1983 Appendix B).
const DISPLAY_HEIGHT_NUMERATOR: i64 = 25;
const DISPLAY_HEIGHT_DENOMINATOR: i64 = 32;
/// The bits of a colour-map entry in a single-value operand's six: an
/// entry is the operand's most significant bits (section 5.3.2.6.2).
const ENTRY_SHIFT: u8 = OPERAND_DATA_BITS - PALETTE_BITS;
/// A colour's primaries in a six-byte operand: twelve bits each, an 8-bit
/// intensity then four zero bits (RAS-6).
const PRIMARY_BITS: u32 = 2 * OPERAND_BYTES as u32;
const PRIMARY_PADDING: u32 = PRIMARY_BITS - 8;
/// The bits of one pel of an `image`: a colour-map entry.
const PEL_BITS: u8 = PALETTE_BITS;
/// TEXT's cursor and drawing point relation "the drawing point leads"
/// (section 5.3.2.3.7), in bits 4 and 3 of the second fixed-format byte:
/// the cursor goes where the drawing point is set, and writing characters
/// leaves the drawing point alone, so text never moves the pen.
const POINT_LEADS: u8 = 0b10 << 2;
/// The moves of an incremental line, two bits each (section 5.3.3.6.4).
const MOVE_ESCAPE: u8 = 0b00;
const MOVE_X: u8 = 0b01;
const MOVE_Y: u8 = 0b10;
const MOVE_BOTH: u8 = 0b11;
/// After the escape: lift or lower the pen, reverse dx, reverse dy.
const MODIFY_PEN: u8 = 0b00;
const MODIFY_REVERSE_X: u8 = 0b01;
const MODIFY_REVERSE_Y: u8 = 0b10;
/// The moves one operand byte holds.
const MOVES_PER_BYTE: usize = 3;
/// Teletext's eight colours, the colour map's entries 0 to 7 at each
/// generation's start (section 7.1; EN 300 706 clause 12.4, Table 30,
/// CLUT 0 entries 0 to 7 at full intensity).
pub const TELETEXT_COLOURS: [[u8; 3]; 8] = [[0, 0, 0], [255, 0, 0], [0, 255, 0], [255, 255, 0], [0, 0, 255], [255, 0, 255], [0, 255, 255], [255, 255, 255]];
/// The entry a generation starts drawing in: teletext's white.
const WHITE: u8 = 7;

/// What a painter draws into.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    /// The backdrop: places are dots of the canvas, and WAIT is allowed.
    Backdrop,
    /// A symbol's macro (section 7.4): written entirely in relative forms
    /// from its own origin, so that it can be stamped anywhere.
    Symbol,
    /// A DRCS character's or a texture's definition: drawn at absolute
    /// places of the definition's own field.
    Pattern,
}

/// The definitions a painter's calls may name by handle (section 7.4),
/// which work package N2 keeps.
pub trait Definitions {
    /// The macro code of the symbol `handle`.
    fn symbol(&self, handle: i32) -> Option<u8>;
    /// The code of the DRCS character `handle`.
    fn drcs(&self, handle: i32) -> Option<u8>;
    /// Which of the programmable patterns A to D, 0 to 3, the texture `handle` is.
    fn texture(&self, handle: i32) -> Option<u8>;
}

/// No definitions: every handle is stale.
pub struct NoDefinitions;

impl Definitions for NoDefinitions {
    fn symbol(&self, _: i32) -> Option<u8> {
        None
    }
    fn drcs(&self, _: i32) -> Option<u8> {
        None
    }
    fn texture(&self, _: i32) -> Option<u8> {
        None
    }
}

/// What is drawn with (section 5.3.2.6).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ink {
    /// Colour mode 1: an entry.
    On(u8),
    /// Colour mode 2: an entry on a background entry.
    Over(u8, u8),
}

/// TEXTURE's line textures (section 5.3.2.4.2), by their bits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Line {
    /// Solid.
    Solid = 0,
    /// Dotted.
    Dotted = 1,
    /// Dashed.
    Dashed = 2,
    /// Dot-dash.
    DotDash = 3,
}

/// The first of TEXTURE's fill patterns that is programmable, A (section
/// 5.3.2.4.3): patterns 0 to 3 are solid, vertical, horizontal and cross.
pub const FIRST_PROGRAMMABLE_FILL: u8 = 4;
/// The last fill pattern, D.
const LAST_FILL: u8 = 7;
/// The last line texture.
const LAST_LINE: i32 = Line::DotDash as i32;

/// What TEXT last set (section 5.3.2.3).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct TextForm {
    /// Character spacing: 0 one field, 1 five quarters, 2 three halves, 3 proportional.
    spacing: u8,
    /// Path: 0 right, 1 left, 2 up, 3 down.
    path: u8,
    /// Rotation, in quarter turns anticlockwise.
    rotation: u8,
    /// Row spacing: 0 one field, 1 five quarters, 2 three halves, 3 two.
    rows: u8,
    /// The character field, in dots and lines.
    width: i32,
    height: i32,
}

/// The form of text on the cell grid (RAS-8).
const GRID_TEXT: TextForm = TextForm { spacing: 0, path: 0, rotation: 0, rows: 0, width: CELL_DOTS, height: CELL_LINES };

/// TEXTURE as last sent: line, fill, highlight, the mask's size.
type Texture = (u8, u8, bool, Option<(i32, i32)>);

/// What the script last asked for.
#[derive(Clone, Debug)]
struct Pen {
    x: i32,
    y: i32,
    ink: Ink,
    /// The logical pel, in dots and lines.
    pel: (i32, i32),
    line: u8,
    fill: u8,
    highlight: bool,
    mask: Option<(i32, i32)>,
}

/// What the receiver holds; `None` where that is not known.
#[derive(Clone, Debug)]
#[expect(clippy::struct_excessive_bools, reason = "four facts about the stream, each read where it is named")]
struct Sent {
    /// Its drawing point, as a six-byte operand's axis values.
    point: Option<(i64, i64)>,
    ink: Option<Ink>,
    pel: Option<(i32, i32)>,
    texture: Option<Texture>,
    text: Option<TextForm>,
    underline: bool,
    reverse: bool,
    /// The PDI set is in use (SO).
    drawing: bool,
    /// A PDI was written and nothing since has ended it.
    open: bool,
}

/// A painter: the pen and what was sent.
#[derive(Clone, Debug)]
pub struct Painter {
    pen: Pen,
    sent: Sent,
    target: Target,
}

/// What a run of a program did beside writing bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Painted {
    /// The tenths of a second of WAIT it wrote.
    pub wait_tenths: u32,
}

/// What makes an outlined shape's opcode its filled form's.
const fn filled_bit(filled: bool) -> u8 {
    if filled { opcode::FILLED } else { 0 }
}

fn range(bound: &str, detail: &str) -> Fault {
    Fault::Range(format!("{bound}: {detail}"))
}

fn kind(detail: &str) -> Fault {
    Fault::Type(detail.to_owned())
}

/// `value` when it is from `low` to `high`, else a `RangeError` naming `bound`.
fn within(value: i32, low: i32, high: i32, bound: &str, what: &str) -> Result<i32, Fault> {
    if (low..=high).contains(&value) { Ok(value) } else { Err(Fault::Range(format!("{bound}: {what} is {low} to {high}, not {value}"))) }
}

/// A colour-map entry.
fn entry(value: i32) -> Result<u8, Fault> {
    within(value, 0, NAPLPS_COLOURS.saturating_sub(1), "NAPLPS_COLOURS", "a colour-map entry").map(|value| u8::try_from(value).unwrap_or(0))
}

/// A place, held to what an operand reaches.
fn place(x: i64, y: i64) -> Result<(i32, i32), Fault> {
    let x =
        i32::try_from(x).ok().filter(|x| (MIN_X..=MAX_X).contains(x)).ok_or_else(|| range("MAX_X", &format!("a place is {MIN_X} to {MAX_X} dots across")))?;
    let y =
        i32::try_from(y).ok().filter(|y| (MIN_Y..=MAX_Y).contains(y)).ok_or_else(|| range("MAX_Y", &format!("a place is {MIN_Y} to {MAX_Y} lines down")))?;
    Ok((x, y))
}

/// The dot (`x`, `y`) of the canvas as a six-byte operand's axis values (RAS-2).
fn steps(x: i32, y: i32, rounding: Rounding) -> (i64, i64) {
    dot_steps(i64::from(x), i64::from(y).saturating_add(i64::from(CANVAS_TOP_LINE)), 1, OPERAND_BYTES, rounding)
}

/// `dots` across as an operand's axis value, rounded down: a size that is
/// a hair under its dots, which the renderer rounds up to exactly them.
fn across(dots: i32) -> i64 {
    unit_steps(i64::from(dots), i64::from(RASTER_WIDTH), OPERAND_BYTES, Rounding::Down)
}

/// `lines` up as an operand's axis value, rounded down as [`across`].
fn up(lines: i32) -> i64 {
    unit_steps(
        i64::from(lines).saturating_mul(DISPLAY_HEIGHT_NUMERATOR),
        DISPLAY_HEIGHT_DENOMINATOR.saturating_mul(i64::from(DISPLAY_LINES)),
        OPERAND_BYTES,
        Rounding::Down,
    )
}

fn operand(across: i64, up: i64, out: &mut Vec<u8>) {
    multi_value_operand(across, up, OPERAND_BYTES, out);
}

/// A step from `from` to `to`, which one operand must hold.
fn step(from: (i64, i64), to: (i64, i64)) -> Result<(i64, i64), Fault> {
    let (dx, dy) = (to.0.saturating_sub(from.0), to.1.saturating_sub(from.1));
    if (AXIS_MIN..=AXIS_MAX).contains(&dx) && (AXIS_MIN..=AXIS_MAX).contains(&dy) {
        Ok((dx, dy))
    } else {
        Err(range("MAX_X", "one step spans more than the screen's width or height, which no operand holds"))
    }
}

/// A colour-map entry as a single-value operand.
const fn entry_operand(entry: u8) -> u8 {
    OPERAND | ((entry << ENTRY_SHIFT) & DATA_MASK)
}

/// A colour operand (section 5.3.2.5.1, Figure 12; RAS-6): each 8-bit
/// primary then four zero bits, two bits to a byte, G R B G R B.
fn colour_operand([red, green, blue]: [u8; 3], out: &mut Vec<u8>) {
    let widen = |primary: u8| u32::from(primary).checked_shl(PRIMARY_PADDING).unwrap_or(0);
    let (red, green, blue) = (widen(red), widen(green), widen(blue));
    let bit = |value: u32, at: u32| u8::from(value.checked_shr(at).unwrap_or(0) & 1 == 1);
    for pair in (0..PRIMARY_BITS / 2).rev() {
        let (low, high) = (pair.saturating_mul(2), pair.saturating_mul(2).saturating_add(1));
        out.push(OPERAND | bit(green, high) << 5 | bit(red, high) << 4 | bit(blue, high) << 3 | bit(green, low) << 2 | bit(red, low) << 1 | bit(blue, low));
    }
}

impl Painter {
    /// A painter of the backdrop at a generation's start, after
    /// [`Painter::start_generation`]'s bytes.
    #[must_use]
    pub const fn backdrop() -> Self {
        Self {
            pen: Pen { x: 0, y: 0, ink: Ink::On(WHITE), pel: (1, 1), line: 0, fill: 0, highlight: false, mask: None },
            sent: Sent {
                point: None,
                ink: None,
                pel: Some((1, 1)),
                texture: Some((0, 0, false, None)),
                text: None,
                underline: false,
                reverse: false,
                drawing: true,
                open: true,
            },
            target: Target::Backdrop,
        }
    }

    /// A painter of a definition's contents (section 7.4), which begin in
    /// the receiver's power-up state with nothing known: a symbol's
    /// drawing point is taken as its own origin, the dot (0, 0), and
    /// every place of it is written as a step from there.
    #[must_use]
    pub fn definition(target: Target) -> Self {
        let mut painter = Self::backdrop();
        painter.target = target;
        painter.sent = Sent { point: None, ink: None, pel: None, texture: None, text: None, underline: false, reverse: false, drawing: false, open: false };
        if target == Target::Symbol {
            painter.sent.point = Some(steps(0, 0, Rounding::Nearest));
        }
        painter
    }

    /// What starts a generation (section 7.1, RAS-6): the PDI set, RESET
    /// of everything, the six-byte DOMAIN with a logical pel of one dot
    /// (without which line and fill textures draw solid: section
    /// 5.3.2.4.2), and teletext's eight colours as entries 0 to 7.
    pub fn start_generation(out: &mut Vec<u8>) {
        out.push(SO);
        out.push(opcode::RESET);
        out.extend_from_slice(&RESET_EVERYTHING);
        domain((1, 1), out);
        for (number, rgb) in (0_u8..).zip(TELETEXT_COLOURS) {
            out.extend_from_slice(&[opcode::SELECT_COLOR, entry_operand(number), opcode::SET_COLOR]);
            colour_operand(rgb, out);
        }
    }

    fn pdi(&mut self, opcode: u8, out: &mut Vec<u8>) {
        if !self.sent.drawing {
            out.push(SO);
            self.sent.drawing = true;
        }
        out.push(opcode);
        self.sent.open = true;
    }

    /// Ends the last PDI, so that the receiver draws it now.
    pub fn end(&mut self, out: &mut Vec<u8>) {
        if self.sent.open {
            out.push(END_PDI);
            self.sent.open = false;
        }
    }

    /// Sets colour-map entry `entry` to `rgb`: SELECT COLOR, then SET
    /// COLOR (sections 5.3.2.5 and 5.3.2.6), which leaves the receiver
    /// drawing in that entry until the pen's ink is next needed.
    pub fn set_colour(&mut self, entry: u8, rgb: [u8; 3], out: &mut Vec<u8>) {
        self.select(Ink::On(entry), out);
        self.pdi(opcode::SET_COLOR, out);
        colour_operand(rgb, out);
    }

    /// BLINK (section 5.3.2.7): entry `from` alternates with `to`, `on`
    /// and `off` tenths of a second, `delay` tenths after the blink
    /// defined before it.
    pub fn blink(&mut self, from: u8, to: u8, [on, off, delay]: [u8; 3], out: &mut Vec<u8>) {
        self.select(Ink::On(from), out);
        self.pdi(opcode::BLINK, out);
        out.extend_from_slice(&[entry_operand(to), OPERAND | (on & DATA_MASK), OPERAND | (off & DATA_MASK), OPERAND | (delay & DATA_MASK)]);
    }

    /// BLINK with no operands: entry `from`'s blinks stop.
    pub fn steady(&mut self, from: u8, out: &mut Vec<u8>) {
        self.select(Ink::On(from), out);
        self.pdi(opcode::BLINK, out);
    }

    fn select(&mut self, ink: Ink, out: &mut Vec<u8>) {
        if self.sent.ink == Some(ink) {
            return;
        }
        self.pdi(opcode::SELECT_COLOR, out);
        match ink {
            Ink::On(entry) => out.push(entry_operand(entry)),
            Ink::Over(entry, background) => out.extend_from_slice(&[entry_operand(entry), entry_operand(background)]),
        }
        self.sent.ink = Some(ink);
    }

    /// Moves the receiver's drawing point to `target`: nothing when it is
    /// there, POINT SET absolute where places are absolute, and in a
    /// symbol POINT SET relative from where it is. With `force` it is
    /// written even so: the cursor follows the drawing point only when a
    /// PDI is executed, and text before has moved the cursor on.
    fn point_to(&mut self, target: (i64, i64), force: bool, out: &mut Vec<u8>) -> Result<(), Fault> {
        if self.sent.point == Some(target) && !force {
            return Ok(());
        }
        if self.target == Target::Symbol {
            let from = self.sent.point.ok_or_else(|| kind("inside a symbol the drawing point is not known after a stamp"))?;
            let (dx, dy) = step(from, target)?;
            self.pdi(opcode::POINT_SET_RELATIVE, out);
            operand(dx, dy, out);
        } else {
            self.pdi(opcode::POINT_SET_ABSOLUTE, out);
            operand(target.0, target.1, out);
        }
        self.sent.point = Some(target);
        Ok(())
    }

    /// Writes what differs between the pen and what was sent, but for the
    /// drawing point: the ink, the logical pel and the textures.
    fn dress(&mut self, out: &mut Vec<u8>) {
        self.select(self.pen.ink, out);
        self.pel(self.pen.pel, out);
        let texture = (self.pen.line, self.pen.fill, self.pen.highlight, self.pen.mask);
        if self.sent.texture != Some(texture) {
            self.pdi(opcode::TEXTURE, out);
            out.push(OPERAND | (self.pen.fill << 3) | (u8::from(self.pen.highlight) << 2) | self.pen.line);
            if let Some((width, height)) = self.pen.mask {
                operand(across(width), up(height), out);
            }
            self.sent.texture = Some(texture);
        }
    }

    fn pel(&mut self, pel: (i32, i32), out: &mut Vec<u8>) {
        if self.sent.pel != Some(pel) {
            if !self.sent.drawing {
                out.push(SO);
                self.sent.drawing = true;
            }
            domain(pel, out);
            self.sent.open = true;
            self.sent.pel = Some(pel);
        }
    }

    /// Dresses the receiver and puts its drawing point on the pen's.
    fn ready(&mut self, out: &mut Vec<u8>) -> Result<(i64, i64), Fault> {
        self.dress(out);
        let at = steps(self.pen.x, self.pen.y, Rounding::Nearest);
        self.point_to(at, false, out)?;
        Ok(at)
    }

    /// The place a relative call's step leads to.
    fn stepped(&self, dx: i32, dy: i32) -> Result<(i32, i32), Fault> {
        place(i64::from(self.pen.x).saturating_add(i64::from(dx)), i64::from(self.pen.y).saturating_add(i64::from(dy)))
    }

    /// A place a call names: in a symbol, a place from its own origin.
    fn named(x: i32, y: i32) -> Result<(i32, i32), Fault> {
        place(i64::from(x), i64::from(y))
    }

    /// Runs `program`, appending its NAPLPS to `out`, which may grow to
    /// `room` bytes, the bound `bound` names. Text is in `language` where
    /// a call names none. On a fault the painter and `out` are as the
    /// calls before it left them: the caller keeps a copy of both to go
    /// back to.
    ///
    /// # Errors
    ///
    /// The first fault of the program: a `RangeError` or a `TypeError`.
    pub fn run(
        &mut self,
        program: &[i32],
        language: &str,
        definitions: &dyn Definitions,
        out: &mut Vec<u8>,
        room: usize,
        bound: &'static str,
    ) -> Result<Painted, Fault> {
        let mut words = Words::new(program);
        let mut painted = Painted::default();
        while !words.is_empty() {
            let number = words.word()?;
            let call = Paint::from_number(number).ok_or_else(|| Fault::Type(format!("the painter has no call numbered {number}")))?;
            self.call(call, &mut words, language, definitions, out, &mut painted)?;
            if out.len() > room {
                return Err(range(bound, "the painting is longer than may be appended"));
            }
        }
        self.end(out);
        if out.len() > room {
            return Err(range(bound, "the painting is longer than may be appended"));
        }
        Ok(painted)
    }

    #[expect(clippy::too_many_lines, reason = "one arm for each call of section 7.3's table, in the table's order")]
    fn call(
        &mut self,
        call: Paint,
        words: &mut Words<'_>,
        language: &str,
        definitions: &dyn Definitions,
        out: &mut Vec<u8>,
        painted: &mut Painted,
    ) -> Result<(), Fault> {
        let relative = self.target == Target::Symbol;
        match call {
            Paint::MOVE_TO => {
                let [x, y] = words.words()?;
                (self.pen.x, self.pen.y) = Self::named(x, y)?;
            }
            Paint::MOVE_BY => {
                let [dx, dy] = words.words()?;
                (self.pen.x, self.pen.y) = self.stepped(dx, dy)?;
            }
            Paint::POINT_AT | Paint::POINT_BY | Paint::LINE_TO | Paint::LINE_BY => {
                let [a, b] = words.words()?;
                let by = matches!(call, Paint::POINT_BY | Paint::LINE_BY);
                let line = matches!(call, Paint::LINE_TO | Paint::LINE_BY);
                let (x, y) = if by { self.stepped(a, b)? } else { Self::named(a, b)? };
                let to = steps(x, y, Rounding::Nearest);
                if line || by || relative {
                    let from = self.ready(out)?;
                    if by || relative {
                        let (dx, dy) = step(from, to)?;
                        self.pdi(if line { opcode::LINE_RELATIVE } else { opcode::POINT_RELATIVE }, out);
                        operand(dx, dy, out);
                    } else {
                        self.pdi(opcode::LINE_ABSOLUTE, out);
                        operand(to.0, to.1, out);
                    }
                } else {
                    self.dress(out);
                    self.pdi(opcode::POINT_ABSOLUTE, out);
                    operand(to.0, to.1, out);
                }
                (self.pen.x, self.pen.y) = (x, y);
                self.sent.point = Some(to);
            }
            Paint::POLYLINE | Paint::POLYLINE_BY => {
                let by = call == Paint::POLYLINE_BY;
                let given = words.pairs(MAX_POINTS, "MAX_POINTS", "a polyline")?;
                let mut places = Vec::with_capacity(given.len());
                let mut last = (self.pen.x, self.pen.y);
                for (a, b) in given {
                    last = if by {
                        place(i64::from(last.0).saturating_add(i64::from(a)), i64::from(last.1).saturating_add(i64::from(b)))?
                    } else {
                        Self::named(a, b)?
                    };
                    places.push(last);
                }
                let mut rest = places.as_slice();
                if !by {
                    // The first point is where the line starts.
                    let Some((first, later)) = rest.split_first() else { return Ok(()) };
                    (self.pen.x, self.pen.y) = *first;
                    rest = later;
                }
                if rest.is_empty() {
                    return Ok(());
                }
                let mut from = self.ready(out)?;
                if by || relative {
                    let mut operands = Vec::with_capacity(rest.len().saturating_mul(usize::from(OPERAND_BYTES)));
                    for (x, y) in rest {
                        let to = steps(*x, *y, Rounding::Nearest);
                        let (dx, dy) = step(from, to)?;
                        operand(dx, dy, &mut operands);
                        from = to;
                    }
                    self.pdi(opcode::LINE_RELATIVE, out);
                    out.extend_from_slice(&operands);
                } else {
                    // SET and LINE's first pair is the start and the first
                    // end (section 5.3.3.2.4); the rest continue as LINE.
                    self.pdi(opcode::SET_LINE_ABSOLUTE, out);
                    operand(from.0, from.1, out);
                    for (index, (x, y)) in rest.iter().enumerate() {
                        if index == 1 {
                            self.pdi(opcode::LINE_ABSOLUTE, out);
                        }
                        from = steps(*x, *y, Rounding::Nearest);
                        operand(from.0, from.1, out);
                    }
                }
                if let Some(end) = rest.last() {
                    (self.pen.x, self.pen.y) = *end;
                }
                self.sent.point = Some(from);
            }
            Paint::RECT | Paint::FILL_RECT | Paint::RECT_BY | Paint::FILL_RECT_BY => {
                let filled = filled_bit(matches!(call, Paint::FILL_RECT | Paint::FILL_RECT_BY));
                let by = matches!(call, Paint::RECT_BY | Paint::FILL_RECT_BY);
                let (x, y, width, height) = if by {
                    let [width, height] = words.words()?;
                    (self.pen.x, self.pen.y, width, height)
                } else {
                    let [x, y, width, height] = words.words()?;
                    (x, y, width, height)
                };
                let width = within(width, 1, CANVAS_WIDTH, "CANVAS_WIDTH", "a rectangle's width")?;
                let height = within(height, 1, CANVAS_HEIGHT, "CANVAS_HEIGHT", "a rectangle's height")?;
                let (x, y) = Self::named(x, y)?;
                // RAS-6: from the dot of its bottom left to the dot of its
                // top right, so that it covers exactly its dots.
                let (left, bottom) = place(i64::from(x), i64::from(y).saturating_add(i64::from(height)).saturating_sub(1))?;
                let (right, _) = place(i64::from(x).saturating_add(i64::from(width)).saturating_sub(1), i64::from(y))?;
                let corner = steps(left, bottom, Rounding::Nearest);
                let size = step(corner, steps(right, y, Rounding::Nearest))?;
                // Afterwards the pen is one rectangle's width along, for
                // adjoining rectangles; a pen moved off the places an
                // operand reaches stays where it was.
                let next = place(i64::from(x).saturating_add(i64::from(width)), i64::from(y)).unwrap_or((x, y));
                self.dress(out);
                if by || relative {
                    self.point_to(corner, false, out)?;
                    self.pdi(opcode::RECTANGLE_OUTLINED | filled, out);
                } else {
                    self.pdi(opcode::SET_RECTANGLE_OUTLINED | filled, out);
                    operand(corner.0, corner.1, out);
                }
                operand(size.0, size.1, out);
                // Section 5.3.3.4.1: the drawing point moves by the width.
                self.sent.point = Some((corner.0.saturating_add(size.0), corner.1));
                (self.pen.x, self.pen.y) = next;
            }
            Paint::POLYGON | Paint::FILL_POLYGON | Paint::POLYGON_BY | Paint::FILL_POLYGON_BY => {
                let filled = filled_bit(matches!(call, Paint::FILL_POLYGON | Paint::FILL_POLYGON_BY));
                let by = matches!(call, Paint::POLYGON_BY | Paint::FILL_POLYGON_BY);
                let given = words.pairs(MAX_POLYGON_VERTICES.saturating_sub(usize::from(by)), "MAX_VERTICES", "a polygon")?;
                let mut last = (self.pen.x, self.pen.y);
                let mut places = Vec::with_capacity(given.len());
                for (a, b) in given {
                    last = if by {
                        place(i64::from(last.0).saturating_add(i64::from(a)), i64::from(last.1).saturating_add(i64::from(b)))?
                    } else {
                        Self::named(a, b)?
                    };
                    places.push(last);
                }
                let mut rest = places.as_slice();
                if !by {
                    let Some((first, later)) = rest.split_first() else { return Ok(()) };
                    (self.pen.x, self.pen.y) = *first;
                    rest = later;
                }
                let start = steps(self.pen.x, self.pen.y, Rounding::Nearest);
                let mut from = start;
                let mut operands = Vec::with_capacity(rest.len().saturating_mul(usize::from(OPERAND_BYTES)));
                for (x, y) in rest {
                    let to = steps(*x, *y, Rounding::Nearest);
                    let (dx, dy) = step(from, to)?;
                    operand(dx, dy, &mut operands);
                    from = to;
                }
                self.dress(out);
                if by || relative {
                    self.point_to(start, false, out)?;
                    self.pdi(opcode::POLYGON_OUTLINED | filled, out);
                } else {
                    self.pdi(opcode::SET_POLYGON_OUTLINED | filled, out);
                    operand(start.0, start.1, out);
                }
                out.extend_from_slice(&operands);
                // Section 5.3.3.5.1: the drawing point is left at the first vertex.
                self.sent.point = Some(start);
            }
            Paint::ARC | Paint::FILL_ARC => {
                let [mx, my, ex, ey] = words.words()?;
                let (middle, end) = (Self::named(mx, my)?, Self::named(ex, ey)?);
                let from = self.ready(out)?;
                let through = steps(middle.0, middle.1, Rounding::Nearest);
                let to = steps(end.0, end.1, Rounding::Nearest);
                let (first, second) = (step(from, through)?, step(through, to)?);
                self.pdi(opcode::ARC_OUTLINED | filled_bit(call == Paint::FILL_ARC), out);
                operand(first.0, first.1, out);
                operand(second.0, second.1, out);
                (self.pen.x, self.pen.y) = end;
                self.sent.point = Some(to);
            }
            Paint::CIRCLE | Paint::DISC => {
                let [cx, cy, radius] = words.words()?;
                let radius = within(radius, 1, CANVAS_WIDTH / 2, "CANVAS_WIDTH", "a circle's radius")?;
                let (cx, cy) = Self::named(cx, cy)?;
                let (left, _) = place(i64::from(cx).saturating_sub(i64::from(radius)), i64::from(cy))?;
                let (right, _) = place(i64::from(cx).saturating_add(i64::from(radius)), i64::from(cy))?;
                let start = steps(left, cy, Rounding::Nearest);
                let across = step(start, steps(right, cy, Rounding::Nearest))?;
                self.dress(out);
                self.point_to(start, false, out)?;
                // Section 5.3.3.3.1: an arc of one step is the circle on
                // that diameter, and the drawing point does not move.
                self.pdi(opcode::ARC_OUTLINED | filled_bit(call == Paint::DISC), out);
                operand(across.0, across.1, out);
            }
            Paint::TRACE | Paint::FILL_TRACE => self.trace(call == Paint::FILL_TRACE, words, out)?,
            Paint::IMAGE => self.image(words, out)?,
            Paint::INK => {
                let [colour] = words.words()?;
                self.pen.ink = Ink::On(entry(colour)?);
            }
            Paint::INK_ON => {
                let [colour, background] = words.words()?;
                self.pen.ink = Ink::Over(entry(colour)?, entry(background)?);
            }
            Paint::PEN => {
                let [width, height] = words.words()?;
                self.pen.pel =
                    (within(width, 1, CANVAS_WIDTH, "CANVAS_WIDTH", "a pen's width")?, within(height, 1, CANVAS_HEIGHT, "CANVAS_HEIGHT", "a pen's height")?);
            }
            Paint::STROKE => {
                let [line] = words.words()?;
                self.pen.line = u8::try_from(line).ok().filter(|line| i32::from(*line) <= LAST_LINE).ok_or_else(|| kind("no such line texture"))?;
            }
            Paint::FILL_STYLE => {
                let [style] = words.words()?;
                self.pen.fill = match u8::try_from(style) {
                    Ok(named) if named < FIRST_PROGRAMMABLE_FILL => named,
                    Ok(FIRST_PROGRAMMABLE_FILL) => {
                        let [handle] = words.words()?;
                        let pattern = definitions.texture(handle).ok_or_else(|| kind("no such texture"))?;
                        FIRST_PROGRAMMABLE_FILL.saturating_add(pattern).min(LAST_FILL)
                    }
                    _ => return Err(kind("no such fill style")),
                };
            }
            Paint::HIGHLIGHT => {
                let [on] = words.words()?;
                self.pen.highlight = match on {
                    0 => false,
                    1 => true,
                    _ => return Err(kind("highlight is on (1) or off (0)")),
                };
            }
            Paint::MASK => {
                let [width, height] = words.words()?;
                self.pen.mask =
                    Some((within(width, 1, MAX_MASK_DOTS, "MASK_SIDE", "a mask's width")?, within(height, 1, MAX_MASK_DOTS, "MASK_SIDE", "a mask's height")?));
            }
            Paint::TEXT => {
                let [row, column, size] = words.words()?;
                let (wide, tall) = match size {
                    0 => (1, 1),
                    1 => (1, 2),
                    2 => (2, 2),
                    _ => return Err(kind("a text size is normal (0), double-height (1) or double-size (2)")),
                };
                let row = within(row, FIRST_TEXT_ROW, LAST_TEXT_ROW.saturating_sub(tall).saturating_add(1), "LAST_TEXT_ROW", "a text row")?;
                let column = within(column, 0, GRID_COLUMNS.saturating_sub(1), "GRID_COLUMNS", "a text column")?;
                let runs = text_of(words, language)?;
                let cells = i64::from(cells(&runs)).saturating_mul(i64::from(wide));
                if i64::from(column).saturating_add(cells) > i64::from(GRID_COLUMNS) {
                    return Err(range("GRID_COLUMNS", "the text passes the row's last column"));
                }
                let form = TextForm { width: CELL_DOTS.saturating_mul(wide), height: CELL_LINES.saturating_mul(tall), ..GRID_TEXT };
                let y = row.saturating_sub(FIRST_TEXT_ROW).saturating_mul(CELL_LINES);
                self.write_text(form, column.saturating_mul(CELL_DOTS), y, false, false, &runs, out)?;
            }
            Paint::TEXT_AT => {
                let [x, y, width, height, rotation, path, spacing, rows, underline, reverse] = words.words()?;
                let two_bits =
                    |value: i32, what: &str| u8::try_from(value).ok().filter(|value| *value <= 3).ok_or_else(|| Fault::Type(format!("no such {what}")));
                let flag = |value: i32, what: &str| match value {
                    0 => Ok(false),
                    1 => Ok(true),
                    _ => Err(Fault::Type(format!("{what} is on (1) or off (0)"))),
                };
                let form = TextForm {
                    spacing: two_bits(spacing, "character spacing")?,
                    path: two_bits(path, "text path")?,
                    rotation: two_bits(rotation, "rotation")?,
                    rows: two_bits(rows, "row spacing")?,
                    width: within(width, 1, CANVAS_WIDTH, "CANVAS_WIDTH", "a character field's width")?,
                    height: within(height, 1, CANVAS_HEIGHT, "CANVAS_HEIGHT", "a character field's height")?,
                };
                let (underline, reverse) = (flag(underline, "underline")?, flag(reverse, "reverse")?);
                let (x, y) = Self::named(x, y)?;
                let runs = text_of(words, language)?;
                self.write_text(form, x, y, underline, reverse, &runs, out)?;
            }
            Paint::GLYPH => {
                let [handle] = words.words()?;
                let code = definitions.drcs(handle).ok_or_else(|| kind("no such DRCS character"))?;
                self.dress(out);
                self.text_form(self.sent.text.unwrap_or(GRID_TEXT), out);
                self.point_to(steps(self.pen.x, self.pen.y, Rounding::Nearest), true, out)?;
                // The DRCS set into G2, the character by a single shift,
                // and the supplementary set back (section 4.3.2).
                out.extend_from_slice(&[ESC, INTO_G2, DRCS_SET, SS2, code, ESC, INTO_G2, SUPPLEMENTARY_SET]);
                self.sent.open = false;
            }
            Paint::STAMP => {
                let [handle, x, y] = words.words()?;
                let code = definitions.symbol(handle).ok_or_else(|| kind("no such symbol"))?;
                (self.pen.x, self.pen.y) = Self::named(x, y)?;
                self.ready(out)?;
                // The macro set into G3 and the macro by a single shift
                // (section 6.2.2). The macro runs in the receiver's state
                // and leaves it as its own contents do: nothing sent
                // before it is known afterwards.
                out.extend_from_slice(&[ESC, INTO_G3, MACRO_SET, SS3, code]);
                self.sent =
                    Sent { point: None, ink: None, pel: None, texture: None, text: None, underline: false, reverse: false, drawing: false, open: false };
            }
            Paint::WAIT => {
                let [tenths] = words.words()?;
                if self.target != Target::Backdrop {
                    return Err(kind("only the backdrop waits: a WAIT inside a definition would do nothing"));
                }
                let tenths = within(tenths, 0, MAX_WAIT_TENTHS, "WAIT_BUDGET_TICKS", "a wait in tenths of a second")?;
                self.pdi(opcode::WAIT, out);
                out.push(WAIT_FORMAT);
                // Each interval byte holds six bits; the receiver sums them.
                let mut left = tenths;
                while left > 0 {
                    let interval = left.min(i32::from(DATA_MASK));
                    out.push(OPERAND | u8::try_from(interval).unwrap_or(0));
                    left = left.saturating_sub(interval);
                }
                painted.wait_tenths = painted.wait_tenths.saturating_add(tenths.unsigned_abs());
            }
        }
        Ok(())
    }

    /// TEXT (section 5.3.2.3), when the form differs from the one sent.
    fn text_form(&mut self, form: TextForm, out: &mut Vec<u8>) {
        if self.sent.text == Some(form) {
            return;
        }
        self.pdi(opcode::TEXT, out);
        out.push(OPERAND | (form.spacing << 4) | (form.path << 2) | form.rotation);
        out.push(OPERAND | POINT_LEADS | form.rows);
        // RAS-8: a field a hair under its dots, rounded down.
        operand(across(form.width), up(form.height), out);
        self.sent.text = Some(form);
    }

    /// Text whose first character field has its top left at the dot (`x`,
    /// `y`): the form, the ink, the drawing point at the field's origin
    /// (its lower left, rounded down as RAS-8 says), the attributes, then
    /// the characters as the engine's picture text encoder writes them
    /// (renderer parity PAR-5), which leaves G0 in use.
    #[expect(clippy::too_many_arguments, reason = "a text call's parts, each named where it is used")]
    fn write_text(&mut self, form: TextForm, x: i32, y: i32, underline: bool, reverse: bool, runs: &[Run], out: &mut Vec<u8>) -> Result<(), Fault> {
        let (left, bottom) = place(i64::from(x), i64::from(y).saturating_add(i64::from(form.height)).saturating_sub(1))?;
        // The domain first: in a definition nothing before it says how
        // long an operand is.
        self.pel(self.pen.pel, out);
        self.text_form(form, out);
        self.select(self.pen.ink, out);
        self.point_to(steps(left, bottom, Rounding::Down), true, out)?;
        for (now, sent, on, off) in [(underline, self.sent.underline, UNDERLINE_ON, UNDERLINE_OFF), (reverse, self.sent.reverse, REVERSE_ON, REVERSE_OFF)] {
            if now != sent {
                out.extend_from_slice(&[ESC, if now { on } else { off }]);
            }
        }
        (self.sent.underline, self.sent.reverse) = (underline, reverse);
        // The encoder's bytes for a stretch begin with its set's
        // designation into G0 and SI, and end with the primary set
        // designated back, which also ends the PDI before them; so the
        // primary set's characters follow as they are, after an SI of
        // their own where no stretch came before.
        let mut shifted_in = false;
        for run in runs {
            match run {
                Run::Sets(stretches) => {
                    picture_text::naplps(stretches, out).map_err(|fault| text_fault(&fault))?;
                    shifted_in |= !stretches.is_empty();
                }
                Run::Primary(bytes) => {
                    if !shifted_in {
                        out.push(SI);
                        shifted_in = true;
                    }
                    out.extend_from_slice(bytes);
                }
            }
        }
        if shifted_in {
            self.sent.drawing = false;
        }
        self.sent.open = false;
        Ok(())
    }

    /// INCREMENTAL LINE and INCREMENTAL POLYGON (sections 5.3.3.6.4 and
    /// 5.3.3.6.5): a chain of moves of one step.
    fn trace(&mut self, filled: bool, words: &mut Words<'_>, out: &mut Vec<u8>) -> Result<(), Fault> {
        let [dx, dy] = words.words()?;
        let dx = within(dx, -CANVAS_WIDTH, CANVAS_WIDTH, "CANVAS_WIDTH", "a trace's step across")?;
        let dy = within(dy, -CANVAS_HEIGHT, CANVAS_HEIGHT, "CANVAS_HEIGHT", "a trace's step down")?;
        let moves = words.list(MAX_TRACE_MOVES, "MAX_VERTICES", "a trace")?;
        // The step as an operand: across, and up for a step down.
        let nearest = |numerator: i64, denominator: i64| unit_steps(numerator, denominator, OPERAND_BYTES, Rounding::Nearest);
        let step_x = nearest(i64::from(dx), i64::from(RASTER_WIDTH));
        let step_y = nearest(i64::from(dy).saturating_mul(DISPLAY_HEIGHT_NUMERATOR), DISPLAY_HEIGHT_DENOMINATOR.saturating_mul(i64::from(DISPLAY_LINES)))
            .saturating_neg();
        // Each move as its two bits, the pen and the directions followed
        // as the receiver follows them.
        let mut codes: Vec<u8> = Vec::with_capacity(moves.len().saturating_mul(2));
        let (mut sign_x, mut sign_y, mut down) = (1_i64, 1_i64, true);
        let (mut moved_x, mut moved_y) = (0_i64, 0_i64);
        for point in moves {
            let character = u32::try_from(*point).ok().and_then(char::from_u32).unwrap_or('\0');
            match character {
                'x' | 'y' | 'd' => {
                    let (code, x, y) = match character {
                        'x' => (MOVE_X, 1, 0),
                        'y' => (MOVE_Y, 0, 1),
                        _ => (MOVE_BOTH, 1, 1),
                    };
                    codes.push(code);
                    moved_x = moved_x.saturating_add(sign_x.saturating_mul(x));
                    moved_y = moved_y.saturating_add(sign_y.saturating_mul(y));
                }
                'u' | 'n' if filled => return Err(kind("a filled trace has no pen to lift")),
                'u' | 'n' => {
                    if down == (character == 'u') {
                        codes.extend_from_slice(&[MOVE_ESCAPE, MODIFY_PEN]);
                        down = !down;
                    }
                }
                '<' => {
                    codes.extend_from_slice(&[MOVE_ESCAPE, MODIFY_REVERSE_X]);
                    sign_x = sign_x.saturating_neg();
                }
                '^' => {
                    codes.extend_from_slice(&[MOVE_ESCAPE, MODIFY_REVERSE_Y]);
                    sign_y = sign_y.saturating_neg();
                }
                _ => return Err(kind("a trace's moves are x, y, d, u, n, < and ^")),
            }
        }
        let end = place(
            i64::from(self.pen.x).saturating_add(moved_x.saturating_mul(i64::from(dx))),
            i64::from(self.pen.y).saturating_add(moved_y.saturating_mul(i64::from(dy))),
        )?;
        let from = self.ready(out)?;
        self.pdi(if filled { opcode::INCREMENTAL_POLYGON } else { opcode::INCREMENTAL_LINE }, out);
        operand(step_x, step_y, out);
        // Three moves to a byte, most significant first; the bits left
        // over are an escape with nothing after it, which does nothing.
        for group in codes.chunks(MOVES_PER_BYTE) {
            let code = |index: usize| group.get(index).copied().unwrap_or(MOVE_ESCAPE);
            out.push(OPERAND | (code(0) << 4) | (code(1) << 2) | code(2));
        }
        if !filled {
            // The receiver's point is where its own steps took it, which
            // is within a fraction of a dot of the pen's place.
            self.sent.point = Some((from.0.saturating_add(moved_x.saturating_mul(step_x)), from.1.saturating_add(moved_y.saturating_mul(step_y))));
            (self.pen.x, self.pen.y) = end;
        }
        Ok(())
    }

    /// FIELD and INCREMENTAL POINT (sections 5.3.3.6.2 and 5.3.3.6.3): a
    /// bitmap of colour-map entries, one dot a pel, its top left at a dot.
    fn image(&mut self, words: &mut Words<'_>, out: &mut Vec<u8>) -> Result<(), Fault> {
        let [x, y, width, height] = words.words()?;
        let width = within(width, 1, CANVAS_WIDTH, "CANVAS_WIDTH", "an image's width")?;
        let height = within(height, 1, NAPLPS_MAX_IMAGE_HEIGHT, "NAPLPS_MAX_IMAGE_HEIGHT", "an image's height")?;
        let (wide, tall) = (usize::try_from(width).unwrap_or(0), usize::try_from(height).unwrap_or(0));
        let pels = words.list(NAPLPS_MAX_IMAGE_PELS, "NAPLPS_MAX_IMAGE_PELS", "an image")?;
        if pels.len() != wide.saturating_mul(tall) {
            return Err(kind("an image has exactly width times height pels"));
        }
        let (x, y) = Self::named(x, y)?;
        // One operand byte's six bits at a time; a row's last byte is
        // padded, because the receiver discards the rest of the byte at
        // a row's end.
        let mut data: Vec<u8> = Vec::with_capacity(pels.len());
        for row in pels.chunks(wide.max(1)) {
            let (mut held, mut bits) = (0_u32, 0_u32);
            for pel in row {
                held = held.checked_shl(u32::from(PEL_BITS)).unwrap_or(0) | u32::from(entry(*pel)?);
                bits = bits.saturating_add(u32::from(PEL_BITS));
                while bits >= DATA_BITS {
                    bits = bits.saturating_sub(DATA_BITS);
                    data.push(OPERAND | (u8::try_from(held.checked_shr(bits).unwrap_or(0) & u32::from(DATA_MASK)).unwrap_or(0)));
                }
            }
            if bits > 0 {
                let rest = held.checked_shl(DATA_BITS.saturating_sub(bits)).unwrap_or(0) & u32::from(DATA_MASK);
                data.push(OPERAND | u8::try_from(rest).unwrap_or(0));
            }
        }
        // The pels are one dot across and one line down each, laid from
        // the image's top left: a field exactly its pels wide, and one
        // step taller than its pels, so that the first row lies inside it.
        let (pel_x, pel_y) = (across(1), up(1));
        let start = steps(x, y, Rounding::Nearest);
        let bottom = (start.0, start.1.saturating_sub(pel_y.saturating_mul(i64::from(height))));
        let size = (pel_x.saturating_mul(i64::from(width)), pel_y.saturating_mul(i64::from(height)).saturating_add(1));
        if !(AXIS_MIN..=AXIS_MAX).contains(&bottom.1) || size.0 > AXIS_MAX || size.1 > AXIS_MAX {
            return Err(range("MAX_Y", "the image lies outside the places an operand reaches"));
        }
        self.select(self.pen.ink, out);
        self.pel((1, 1), out);
        self.point_to(bottom, false, out)?;
        // FIELD with a size alone takes the drawing point as its origin.
        self.pdi(opcode::FIELD, out);
        operand(size.0, size.1, out);
        // It leaves the drawing point there; the first pel is the top left's.
        self.point_to(start, false, out)?;
        self.pdi(opcode::INCREMENTAL_POINT, out);
        out.push(OPERAND | PEL_BITS);
        out.extend_from_slice(&data);
        // Section 5.3.3.6.3: the drawing point is left at the field's origin.
        self.sent.point = Some(bottom);
        Ok(())
    }
}

/// DOMAIN (section 5.3.2.2): six-byte operands and the logical pel, which
/// hangs right and down from the drawing point, as a rectangle's dots do
/// from its top left: a positive width and a negative height.
fn domain((width, height): (i32, i32), out: &mut Vec<u8>) {
    out.extend_from_slice(&[opcode::DOMAIN, DOMAIN_FORMAT]);
    operand(across(width), up(height).saturating_neg(), out);
}

/// A stretch of a text as it is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Run {
    /// Codes of the provided sets, as the engine's picture text encoder
    /// chose, ordered and joined them.
    Sets(Vec<Stretch>),
    /// Characters of the primary set (ANSI X3.110-1983 section 4.3.2,
    /// Table 1: ASCII), which the receiver draws from its stroke font.
    Primary(Vec<u8>),
}

/// A text call's language and text, as runs.
fn text_of(words: &mut Words<'_>, language: &str) -> Result<Vec<Run>, Fault> {
    let named = words.string(crate::limits::MAX_LANGUAGE_BYTES, "MAX_LANGUAGE_BYTES", "a language tag")?;
    let text = words.string(MAX_TEXT_CHARS, "MAX_TEXT_CHARS", "a text")?;
    runs(&text, if named.is_empty() { language } else { &named })
}

/// Whether `character` is one of the primary set's 94: printable ASCII
/// but the space, which every provided set holds.
const fn in_primary_set(character: char) -> bool {
    matches!(character, '!'..='~')
}

/// `text` in `language` as it is written (section 7.3, "Text and
/// languages"): in the 55 provided sets (REQ-6668) as the engine's
/// picture text encoder chooses, orders and joins them for the language
/// (renderer parity PAR-3), one Noto cell a character (RAS-8); and, for
/// the ASCII characters the language's sets do not hold (a teletext
/// national option gives some of ASCII's places to its own letters), in
/// the primary set.
///
/// # Errors
///
/// A `TypeError` naming the character neither holds, for a control
/// character, or for a language the engine has not; a `RangeError` for a
/// text that is too long.
pub fn runs(text: &str, language: &str) -> Result<Vec<Run>, Fault> {
    let tag = (!language.is_empty()).then_some(language);
    let encode = |part: &str| picture_text::stretches(part, tag, false);
    let fault = match encode(text) {
        Ok(stretches) => return Ok(vec![Run::Sets(stretches)]),
        Err(fault) => fault,
    };
    if !matches!(fault, PictureTextError::Unshown { .. }) {
        return Err(text_fault(&fault));
    }
    // Which of the text's characters the sets do not show, each asked
    // once: at most the text's own length of single characters.
    let mut asked: Vec<(char, bool)> = Vec::new();
    let mut shown = |character: char| {
        if let Some((_, shown)) = asked.iter().find(|(known, _)| *known == character) {
            return *shown;
        }
        let shown = !matches!(encode(character.encode_utf8(&mut [0; 4])), Err(PictureTextError::Unshown { .. }));
        asked.push((character, shown));
        shown
    };
    let mut runs: Vec<Run> = Vec::new();
    let mut part = String::new();
    let close = |part: &mut String, runs: &mut Vec<Run>| -> Result<(), Fault> {
        if !part.is_empty() {
            runs.push(Run::Sets(encode(part).map_err(|fault| text_fault(&fault))?));
            part.clear();
        }
        Ok(())
    };
    for character in text.chars() {
        if shown(character) {
            part.push(character);
        } else if in_primary_set(character) {
            close(&mut part, &mut runs)?;
            let byte = u8::try_from(character).unwrap_or(b' ');
            match runs.last_mut() {
                Some(Run::Primary(bytes)) => bytes.push(byte),
                _ => runs.push(Run::Primary(vec![byte])),
            }
        } else {
            return Err(Fault::Type(format!("text: no character set of the language shows U+{:04X}", u32::from(character))));
        }
    }
    close(&mut part, &mut runs)?;
    Ok(runs)
}

/// The character cells `runs` advance: a letter and its mark are one.
#[must_use]
pub fn cells(runs: &[Run]) -> u32 {
    runs.iter().fold(0_u32, |sum, run| {
        sum.saturating_add(match run {
            Run::Sets(stretches) => picture_text::measure(stretches).cells,
            Run::Primary(bytes) => u32::try_from(bytes.len()).unwrap_or(u32::MAX),
        })
    })
}

fn text_fault(fault: &PictureTextError) -> Fault {
    match fault {
        PictureTextError::TooLong { .. } => range("MAX_TEXT_CHARS", "the text is too long"),
        other => Fault::Type(format!("text: {other}")),
    }
}
