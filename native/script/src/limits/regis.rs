//! The bounds of the ReGIS API (script-api.md sections 6 and 10.6; work
//! package R1), each defined once, here, or taken by name from the
//! renderer's own limits (`regis::limits`) and the link's. The host holds
//! one page to far less than the renderer's own caps, so that one page
//! cannot hold the renderer's whole memory; the assertions below hold
//! each of the host's bounds inside the renderer's.

/// The entries of the ReGIS colour map (the VT340's sixteen; `S(M n(…))`,
/// the manual's chapter 2, "Output Mapping").
pub const REGIS_COLOURS: usize = 16;
/// The most numbered drawings the two contexts of one page may hold
/// together (the host's bound, section 10.6).
pub const REGIS_MAX_DRAWINGS: usize = 2048;
/// The drawings among them that only the channel context may hold, so
/// that neither level can starve the other (section 3.1).
pub const CHANNEL_SHARE_DRAWINGS: usize = 64;
/// The most bytes of one drawing's contents, as the host's encoder writes
/// them.
pub const REGIS_MAX_CONTENTS_BYTES: usize = 65_536;
/// The most bytes of contents all of a page's drawings hold together.
pub const REGIS_MAX_TOTAL_CONTENTS_BYTES: usize = 1024 * 1024;
/// The bytes of contents among them that only the channel context may
/// hold: one drawing's worth (this work package's choice, as
/// [`CHANNEL_SHARE_DRAWINGS`] is the specification's for the table).
pub const CHANNEL_SHARE_CONTENTS_BYTES: usize = REGIS_MAX_CONTENTS_BYTES;
/// The first and the last drawing number the host gives out
/// (`regis-v2.md`, "The D command": 1 to 65,535).
pub const FIRST_DRAWING_NUMBER: u16 = 1;
/// See [`FIRST_DRAWING_NUMBER`].
pub const LAST_DRAWING_NUMBER: u16 = regis::limits::MAX_DRAWING_NUMBER;
/// The last number the page context's drawings take; the channel
/// context's take the numbers above it, so that the two never meet and
/// neither can use up the other's.
pub const LAST_PAGE_DRAWING_NUMBER: u16 = LAST_DRAWING_NUMBER / 2;
/// The deepest a chain of parents may be (the renderer's cap, by name).
pub const MAX_DRAWING_DEPTH: usize = regis::limits::MAX_DRAWING_DEPTH;
/// The fastest paint rate (the renderer's, by name).
pub const MAX_PAINT_RATE: i32 = regis::limits::MAX_PAINT_RATE as i32;
/// What operation `RG_DRAWING` and property `PAINT_RATE` give for a paint
/// rate that is unset (section 10.2: "paint rate (−1 unset)").
pub const PAINT_RATE_UNSET: i32 = -1;
/// The layers of a picture (the renderer's, by name).
pub const LAYERS: i32 = regis::limits::LAYERS as i32;
/// The layer a drawing is in unless it says otherwise (the renderer's).
pub const DEFAULT_LAYER: i32 = regis::limits::DEFAULT_LAYER as i32;
/// The overlay layer: the only one the channel context may draw in
/// (section 3.1).
pub const OVERLAY_LAYER: i32 = LAYERS - 1;
/// The least and the greatest value of a position, a depth and a
/// coordinate inside a drawing: ReGIS's signed 16 bits (the renderer's).
pub const MIN_COORDINATE: i32 = regis::limits::MIN_COORDINATE;
/// See [`MIN_COORDINATE`].
pub const MAX_COORDINATE: i32 = regis::limits::MAX_COORDINATE;
/// The most entries of a frame list (the renderer's, by name).
pub const MAX_OBJECT_FRAMES: usize = regis::limits::MAX_OBJECT_FRAMES;
/// The highest frame an animation operation names (the rule table's).
pub const MAX_FRAME: i32 = neoteletext_rules::limits::MAX_FRAME as i32;
/// The longest period of a cycle, in ticks (the rule table's).
pub const MAX_CYCLE_PERIOD: i32 = neoteletext_rules::limits::MAX_CYCLE_PERIOD as i32;
/// The highest forced look: 0 none, 1 hover, 2 pressed, 3 alternative
/// (the rule table's).
pub const MAX_LOOK: i32 = neoteletext_rules::limits::MAX_LOOK as i32;
/// The most drawings that hold animation state at once (the renderer's).
pub const MAX_ANIMATED_DRAWINGS: usize = regis::limits::MAX_ANIMATED_DRAWINGS;
/// The most drawings cycling their frames at once (the renderer's).
pub const MAX_CYCLING_OBJECTS: usize = regis::limits::MAX_CYCLING_OBJECTS;
/// The most animation operations one step's batch carries (the link's).
pub const MAX_ANIMATE_OPERATIONS: usize = neoteletext_rules::limits::MAX_ANIMATE_OPERATIONS;
/// The most drawings whose painting is watched (the link's bound, which
/// is the renderer's).
pub const MAX_WATCHED_PAINTINGS: usize = protocol::script::MAX_WATCHED_PAINTINGS;
/// The most drawings whose bounds are reported with each step (the link's).
pub const MAX_TRACKED_DRAWINGS: usize = protocol::script::MAX_TRACKED_DRAWINGS;
/// The most drawings that take the wheel (the link's).
pub const MAX_WHEEL_THINGS: usize = protocol::script::batch::MAX_WHEEL_THINGS;
/// The most whole numbers of one painter program (section 10.6): as many
/// as one host call's payload holds.
pub const MAX_PAINTER_WORDS: usize = 65_536;
/// The most points of one `polyline` or `polygon` (section 10.6).
pub const MAX_POINTS: usize = 1024;
/// The most vertices one `fill` may trace (the renderer's, by name).
pub const MAX_FILL_VERTICES: usize = regis::limits::MAX_FILL_VERTICES;
/// The redefinitions of one colour entry that take effect in a second:
/// three flashes, the most that is safe for viewers (section 10.5). It is
/// one rule for every renderer, defined once, in the library's limits;
/// a later value waits for the next tick the rate allows.
pub use super::MAX_COLOUR_SETS_PER_SECOND;
/// The longest name `colours.define` takes, in bytes (this work package's
/// choice: the specification bounds no name).
pub const MAX_COLOUR_NAME_BYTES: usize = 32;
/// The greatest hue, lightness and saturation ReGIS takes (the manual's
/// chapter 2: hue in degrees, the others in percent).
pub const MAX_HUE: i32 = 360;
/// See [`MAX_HUE`].
pub const MAX_PERCENT: i32 = 100;
/// The greatest colour a `#rrggbb` form gives: 24 bits.
pub const MAX_RGB24: i32 = 0x00FF_FFFF;

const _: () = assert!(CHANNEL_SHARE_DRAWINGS < REGIS_MAX_DRAWINGS && CHANNEL_SHARE_CONTENTS_BYTES < REGIS_MAX_TOTAL_CONTENTS_BYTES);
// Every drawing has a number, twice over: a number freed in a step is
// not given out again in that step, so each context's range holds its
// drawings before the step and after it.
const _: () = assert!(2 * REGIS_MAX_DRAWINGS < (LAST_PAGE_DRAWING_NUMBER - FIRST_DRAWING_NUMBER) as usize);
const _: () = assert!(2 * CHANNEL_SHARE_DRAWINGS < (LAST_DRAWING_NUMBER - LAST_PAGE_DRAWING_NUMBER) as usize);
// What the host lets a page hold, the renderer holds: its caps on the
// drawings' records (DEC-6850), on their contents and on one batch are
// never reached, so those three commit errors cannot happen.
const _: () =
    assert!(REGIS_MAX_DRAWINGS * (regis::limits::DRAWING_RECORD_BYTES + regis::limits::DRAWING_EXTRAS_BYTES) <= regis::limits::MAX_DRAWING_RECORD_BYTES);
const _: () = assert!(REGIS_MAX_TOTAL_CONTENTS_BYTES <= regis::limits::MAX_DRAWING_CONTENT_BYTES);
const _: () = assert!(super::MAX_BATCH_BYTES <= regis::limits::MAX_STAGED_BYTES);
// A painter program is one host call's payload, four bytes a number.
const _: () = assert!(MAX_PAINTER_WORDS * size_of::<i32>() <= super::MAX_HOST_BYTES);
// The link's bounds are the renderer's.
const _: () = assert!(MAX_WATCHED_PAINTINGS == regis::limits::MAX_WATCHED_PAINTINGS);
const _: () = assert!(super::MAX_LABEL_BYTES == regis::limits::MAX_LABEL_BYTES);
const _: () = assert!(super::MAX_STEP_CYCLE_ENDS == MAX_CYCLING_OBJECTS);
const _: () = assert!(MAX_OBJECT_FRAMES == neoteletext_rules::limits::MAX_FRAME as usize + 1);
