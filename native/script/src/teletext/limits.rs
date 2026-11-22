//! The bounds of the teletext API (script-api.md section 10.6), each
//! defined once: the engine's by its own name, the link's by the name the
//! messages give it, and the few that are this API's own.

use neoteletext_core::consts::{COLOUR_MAP_ENTRIES, COLOURS, COLUMNS, DOUBLE_HEIGHT_LAST_ROW, PACKET_FIRST_ROW, PACKET_LAST_TEXT_ROW};
use protocol::script::batch::{
    FIRST_DEFINABLE_ENTRY, FIRST_SCRIPT_ROW, LAST_COLOUR_ENTRY, LAST_SCRIPT_ROW, MAX_DEFINABLE_COLOURS, MAX_LOCAL_TRIPLETS, MAX_RGB12, MAX_SCRIPT_ROWS,
};

/// `value` as a signed whole number, at compile time; a constant that did
/// not fit would fail the build here.
const fn wide(value: usize) -> i32 {
    assert!(value <= i32::MAX as usize);
    #[expect(clippy::cast_possible_truncation, clippy::cast_possible_wrap, reason = "the assertion above holds it to 31 bits")]
    let wide = value as i32;
    wide
}

/// The first row a script writes: row 0 is the receiver's (section 5).
pub const FIRST_ROW: i32 = FIRST_SCRIPT_ROW as i32;
/// The first row, as the link's rows are numbered.
pub const FIRST_ROW_BYTE: u8 = FIRST_SCRIPT_ROW;
/// The last row a script writes: row 24 is the receiver's.
pub const LAST_ROW: i32 = LAST_SCRIPT_ROW as i32;
/// The rows of the page plane.
pub const PLANE_ROWS: usize = MAX_SCRIPT_ROWS;
/// The last column of a row.
pub const LAST_COLUMN: i32 = wide(COLUMNS - 1);
/// The codes `page.blit` takes: the whole plane, row by row (section 5.1).
pub const BLIT_CODES: usize = PLANE_ROWS * COLUMNS;
/// The last row double height may start on, so that the row it takes is a
/// row of the plane: the engine's, which also keeps it out of row 23.
pub const LAST_DOUBLE_HEIGHT_ROW: i32 = wide(DOUBLE_HEIGHT_LAST_ROW);
/// The triplets of a page's local enhancement data: sixteen packets X/26
/// of thirteen (EN 300 706 clause 12.3.1), the engine's `LOCAL_TRIPLETS`.
pub const LOCAL_TRIPLETS: usize = MAX_LOCAL_TRIPLETS;
/// The most controls a page has: the engine's (decision E.127).
pub const MAX_CONTROLS: usize = neoteletext_core::controls::MAX_CONTROLS;
/// The greatest Level 1 colour: the eight of Table 26.
pub const LAST_COLOUR8: i32 = wide(COLOURS - 1);
/// The greatest Colour Map entry (EN 300 706 clause 12.4).
pub const LAST_ENTRY: i32 = LAST_COLOUR_ENTRY as i32;
/// The first Colour Map entry a script defines: those of CLUTs 2 and 3
/// (clause 9.4.2.2; section 5.5).
pub const FIRST_DEFINABLE: i32 = FIRST_DEFINABLE_ENTRY as i32;
/// The colours a script defines.
pub const TELETEXT_DEFINABLE_COLOURS: usize = MAX_DEFINABLE_COLOURS;
/// The greatest colour: four bits each of red, green and blue.
pub const LAST_RGB12: i32 = MAX_RGB12 as i32;
/// The most redefinitions of one colour entry in a second: three flashes
/// (section 10.5; the security review's finding S-10, TODO-7276: more
/// than three flashes a second is unsafe for viewers).
pub const MAX_COLOUR_SETS_PER_SECOND: usize = 6;
/// The most characters of a text a script gives (section 10.6).
pub const MAX_TEXT_CHARS: usize = 1024;
/// The forced looks of a control: none, hover and pressed (section 5.4;
/// the engine's decision E.127 gives a teletext control two looks).
pub const LAST_CONTROL_LOOK: i32 = 2;

// The rows the link carries are the rows the engine's controls and text
// may lie on, the definable colours are the Colour Map's upper half, and
// a control's number fits the rule table's.
const _: () = assert!(FIRST_SCRIPT_ROW == PACKET_FIRST_ROW && LAST_SCRIPT_ROW == PACKET_LAST_TEXT_ROW);
const _: () = assert!(LAST_COLOUR_ENTRY as usize == COLOUR_MAP_ENTRIES - 1 && MAX_DEFINABLE_COLOURS == COLOUR_MAP_ENTRIES / 2);
const _: () = assert!(MAX_CONTROLS == neoteletext_rules::limits::MAX_CONTROL as usize);
const _: () = assert!(LAST_CONTROL_LOOK <= neoteletext_rules::limits::MAX_LOOK as i32);
const _: () = assert!(LAST_DOUBLE_HEIGHT_ROW < LAST_ROW);
