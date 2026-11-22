//! Every bound of the NAPLPS API (script-api.md section 10.6), each
//! defined once: the renderer's own by its name, and the host's.

use naplps::limits::{DISPLAY_FIRST_LINE, DISPLAY_LINES, MASK_SIDE, MAX_VERTICES, RASTER_ROW_LINES, RASTER_WIDTH, TENTH_TICKS, WAIT_BUDGET_TICKS};

/// A renderer's `u32` as the whole number a script's argument is.
const fn whole(value: u32) -> i32 {
    assert!(value <= i32::MAX as u32);
    #[expect(clippy::cast_possible_wrap, reason = "the assertion above holds it to 31 bits")]
    let whole = value as i32;
    whole
}

/// The canvas a script draws on, in dots: the renderer's receiver canvas
/// (RAS-11), the raster's lines of teletext rows 1 to 23.
pub const CANVAS_WIDTH: i32 = whole(RASTER_WIDTH);
/// See [`CANVAS_WIDTH`].
pub const CANVAS_HEIGHT: i32 = whole(DISPLAY_LINES);
/// The raster line of the canvas's first line (RAS-11).
pub const CANVAS_TOP_LINE: i32 = whole(DISPLAY_FIRST_LINE);
/// The places a coordinate may name: a canvas's width to the left of the
/// canvas and its height below it, and the rows above it that a six-byte
/// operand still reaches, so that a shape may begin or end off the
/// picture. The module's tests show that `dot_steps` holds none of these
/// to the operand's range.
pub const MIN_X: i32 = -CANVAS_WIDTH;
/// See [`MIN_X`].
pub const MAX_X: i32 = CANVAS_WIDTH - 1;
/// See [`MIN_X`]: five text rows above the canvas.
pub const MIN_Y: i32 = -5 * whole(RASTER_ROW_LINES);
/// See [`MIN_X`].
pub const MAX_Y: i32 = 2 * CANVAS_HEIGHT - 1;

/// The colour map's entries: the browser's decoder draws NAPLPS with four
/// address bits (`decoder/src/naplps.rs`, `PALETTE_BITS`).
pub const PALETTE_BITS: u8 = 4;
/// See [`PALETTE_BITS`].
pub const NAPLPS_COLOURS: i32 = 1 << PALETTE_BITS;
/// The first entry a script defines: those below are teletext's eight
/// colours, set at each generation's start (section 7.1).
pub const FIRST_DEFINABLE_COLOUR: i32 = 8;
/// The longest name of a defined colour, in bytes.
pub const MAX_COLOUR_NAME_BYTES: usize = 32;
/// The most redefinitions of one colour entry in a second (section 10.5),
/// which every renderer holds: the script library's one definition.
pub use crate::limits::MAX_COLOUR_SETS_PER_SECOND;
/// The most blink processes (the renderer's bound, by name).
pub const MAX_BLINKS: usize = naplps::limits::MAX_BLINKS;
/// The longest interval of a blink, in tenths of a second: an operand
/// byte's six bits (ANSI X3.110-1983 section 5.3.2.7).
pub const MAX_BLINK_TENTHS: i32 = 63;

/// The ticks between two generations (section 10.6).
pub const NAPLPS_REPAINT_TICKS: u64 = 30;
/// The most NAPLPS bytes one callback appends (section 10.6).
pub const NAPLPS_MAX_BYTES_PER_CALLBACK: usize = 65_536;
/// The most NAPLPS bytes appended in any second (section 10.6).
pub const NAPLPS_MAX_BYTES_PER_SECOND: usize = 262_144;
/// The most pels of one `image` (section 10.6).
pub const NAPLPS_MAX_IMAGE_PELS: usize = 16_384;
/// The tallest `image`, in lines. An image's rows are stepped by a whole
/// number of operand steps, 222 where a line is 222.6, so each row drifts
/// 0.0027 of a line from its dot; at this many rows that is 0.35 of a
/// line, under the half that would move a row to its neighbour.
pub const NAPLPS_MAX_IMAGE_HEIGHT: i32 = 128;
/// The most words of a painter program (section 10.6).
pub const MAX_PAINTER_WORDS: usize = 65_536;
/// The most points of a polyline (section 10.6).
pub const MAX_POINTS: usize = 1_024;
/// The most vertices of a polygon (the renderer's bound, by name).
pub const MAX_POLYGON_VERTICES: usize = MAX_VERTICES;
/// The most moves of a trace: a filled trace is a polygon of a vertex for
/// each move and its start.
pub const MAX_TRACE_MOVES: usize = MAX_VERTICES - 1;
/// The most characters of one text call: the engine's bound on a run of
/// picture text, by name.
pub const MAX_TEXT_CHARS: usize = neoteletext_formats::picture_text::MAX_PART_CHARACTERS;
/// The longest side of a texture's pattern, in dots (the renderer's
/// bound on a mask, by name).
pub const MAX_MASK_DOTS: i32 = whole(MASK_SIDE);
/// The ticks of WAIT a generation plays (the renderer's bound, by name),
/// and the same in the tenths of a second a WAIT is written in.
pub const WAIT_BUDGET: u32 = WAIT_BUDGET_TICKS;
/// See [`WAIT_BUDGET`].
pub const MAX_WAIT_TENTHS: i32 = whole(WAIT_BUDGET_TICKS / TENTH_TICKS);

/// The text grid: teletext's 40 columns of 12 dots (RAS-8).
pub const GRID_COLUMNS: i32 = 40;
/// A cell's width in dots.
pub const CELL_DOTS: i32 = CANVAS_WIDTH / GRID_COLUMNS;
/// A cell's height in lines.
pub const CELL_LINES: i32 = whole(RASTER_ROW_LINES);
/// The first and last text rows of the canvas: teletext rows 1 to 23.
pub const FIRST_TEXT_ROW: i32 = CANVAS_TOP_LINE / CELL_LINES;
/// See [`FIRST_TEXT_ROW`].
pub const LAST_TEXT_ROW: i32 = (CANVAS_TOP_LINE + CANVAS_HEIGHT) / CELL_LINES - 1;

const _: () = assert!(CELL_DOTS * GRID_COLUMNS == CANVAS_WIDTH && FIRST_TEXT_ROW == 1 && LAST_TEXT_ROW == 23);
// One paint fits a step's output, and a second's paints a second's.
const _: () = assert!(NAPLPS_MAX_BYTES_PER_CALLBACK <= crate::limits::MAX_BATCH_BYTES && NAPLPS_MAX_BYTES_PER_CALLBACK <= NAPLPS_MAX_BYTES_PER_SECOND);
// A painter program is one payload of a host call.
const _: () = assert!(MAX_PAINTER_WORDS * size_of::<i32>() <= crate::limits::MAX_HOST_BYTES);
// An image's rows of four bits a pel, each padded to a whole byte, fit one PDI.
const _: () = assert!(NAPLPS_MAX_IMAGE_PELS + NAPLPS_MAX_IMAGE_HEIGHT as usize <= naplps::limits::MAX_PDI_BYTES);
// A polygon's operands, six bytes a vertex, fit one PDI.
const _: () = assert!(MAX_POLYGON_VERTICES * 6 < naplps::limits::MAX_PDI_BYTES);

/// The codes a macro or a DRCS character is named by, and so the most
/// symbols and the most DRCS characters (the renderer's bound, by name).
pub const DEFINABLE_CODES: usize = naplps::limits::DEFINABLE_CODES;
/// The longest contents of one symbol, DRCS character or texture (the
/// renderer's bound on a macro, by name).
pub const MAX_MACRO_BYTES: usize = naplps::limits::MAX_MACRO_BYTES;
/// The most bytes of every symbol's contents together (the renderer's
/// bound, by name).
pub const MAX_ALL_MACRO_BYTES: usize = naplps::limits::MAX_ALL_MACRO_BYTES;
/// The most bytes of every DRCS character's and texture's contents
/// together: eight of the longest. The host keeps them to send again
/// with each new generation, so they are part of every repaint's cost.
pub const NAPLPS_MAX_PATTERN_BYTES: usize = 8 * MAX_MACRO_BYTES;
/// The programmable fill patterns, A to D (the renderer's bound, by name).
pub const TEXTURE_MASKS: usize = naplps::limits::TEXTURE_MASKS;
/// The most actors and regions of a page (the renderer's bound, by name).
pub const MAX_OBJECTS: usize = naplps::limits::MAX_OBJECTS;
/// The most frames of an actor (the renderer's bound, by name).
pub const MAX_OBJECT_FRAMES: usize = naplps::limits::MAX_OBJECT_FRAMES;
/// The most objects cycling their frames at once (the renderer's bound, by name).
pub const MAX_CYCLING_OBJECTS: usize = naplps::limits::MAX_CYCLING_OBJECTS;
/// The last frame an object shows (the renderer's bound, by name).
pub const MAX_FRAME: i32 = naplps::limits::MAX_FRAME_NUMBER as i32;
/// The longest period of a frame cycle, in ticks (the rule table's bound, by name).
pub const MAX_CYCLE_PERIOD: i32 = neoteletext_rules::limits::MAX_CYCLE_PERIOD as i32;
/// The most animation operations one step's batch holds (the rule table's bound, by name).
pub const MAX_ANIMATE_OPERATIONS: usize = neoteletext_rules::limits::MAX_ANIMATE_OPERATIONS;
/// The channel context's share of the cast and of the symbols (section
/// 10.6), so that neither level can starve the other.
pub const CHANNEL_SHARE_ACTORS: usize = 8;
/// See [`CHANNEL_SHARE_ACTORS`].
pub const CHANNEL_SHARE_SYMBOLS: usize = 16;

const _: () = assert!(CHANNEL_SHARE_ACTORS < MAX_OBJECTS && CHANNEL_SHARE_SYMBOLS < DEFINABLE_CODES);
// The objects of one record are the batch's, and the frames an object's.
const _: () = assert!(MAX_OBJECTS == protocol::script::batch::MAX_NAPLPS_OBJECTS && MAX_OBJECT_FRAMES == protocol::script::batch::MAX_OBJECT_FRAMES);
// A symbol's handle holds its slot in eight bits.
const _: () = assert!(DEFINABLE_CODES <= 256);
