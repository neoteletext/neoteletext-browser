//! The bounds of teletext sprites (script-api.md sections 5.3 and 10.6),
//! each defined once: the engine's by its own name, the link's by the
//! name the messages give it, and the few that are this API's own.
//! Teletext's object model is surfaced as it is, so its real limits are
//! here by the standard's clauses, not rounder figures.

use neoteletext_core::consts::COLUMNS;
use protocol::script::batch::{MAX_OBJECT_TRIPLETS, MAX_SPRITE_COLUMN, MAX_SPRITE_HEIGHT, MAX_SPRITE_ROW, MAX_SPRITE_WIDTH, MAX_TELETEXT_OBJECTS};
use protocol::script::teletext::CONTROL_HANDLE_BASE;

pub use neoteletext_core::limits::{LAST_FRAME, LAST_LAYER, MAX_ANIMATED_OBJECTS, MAX_CYCLING_OBJECTS, MAX_OBJECT_FRAMES, MAX_SPRITES};
pub use neoteletext_rules::limits::{KEEP_LAYER, MAX_ANIMATE_OPERATIONS, MAX_CYCLE_PERIOD};
pub use protocol::script::sprites::{
    CHANNEL_SHARE_OBJECT_SUBPAGES, CHANNEL_SHARE_SPRITES, MAX_OBJECT_BODY, OBJECT_FRAMING_TRIPLETS, OBJECT_SUBPAGES, OBJECTS_PER_SUBPAGE,
    PAGE_SHARE_OBJECT_SUBPAGES, SUBPAGE_TRIPLETS,
};

use super::super::limits::{MAX_CONTROLS, PLANE_ROWS};

/// `value` as a signed whole number, at compile time; a constant that did
/// not fit would fail the build here.
const fn wide(value: usize) -> i32 {
    assert!(value <= i32::MAX as usize);
    #[expect(clippy::cast_possible_truncation, clippy::cast_possible_wrap, reason = "the assertion above holds it to 31 bits")]
    let wide = value as i32;
    wide
}

/// The sprites the page context may have: the page's, less the channel
/// context's share.
pub const PAGE_SHARE_SPRITES: usize = MAX_SPRITES - CHANNEL_SHARE_SPRITES;
/// The widest block, in cells: a row.
pub const MAX_BLOCK_COLUMNS: i32 = wide(COLUMNS);
/// The tallest block, in cells: the rows a script draws on, 1 to 23.
pub const MAX_BLOCK_ROWS: i32 = wide(PLANE_ROWS);
/// The most frames of a sprite, as a host call's whole number.
pub const MAX_FRAMES: i32 = wide(MAX_OBJECT_FRAMES);
/// An object is at most this many triplets long, its Object Definition
/// triplet and Termination Marker included (EN 300 706 clause 13.1): the
/// engine's `OBJECT_BODY_TRIPLETS`.
pub const OBJECT_BODY_TRIPLETS: usize = SUBPAGE_TRIPLETS;
/// The triplets of the whole Object page: what every frame of every
/// sprite of both contexts shares.
pub const OBJECT_PAGE_TRIPLETS: usize = OBJECT_SUBPAGES * SUBPAGE_TRIPLETS;
/// The farthest column of a sprite's origin, either way (the `SPRITES`
/// record, ANIM-7).
pub const SPRITE_COLUMN_REACH: i32 = MAX_SPRITE_COLUMN as i32;
/// The farthest row of a sprite's origin, either way.
pub const SPRITE_ROW_REACH: i32 = MAX_SPRITE_ROW as i32;
/// The first layer a sprite may be in: layer 0 is the page itself
/// (ANIM-12).
pub const FIRST_SPRITE_LAYER: i32 = 1;
/// The last layer, the overlay's: the only one the channel context's
/// sprites may be in (section 3.1).
pub const LAST_SPRITE_LAYER: i32 = LAST_LAYER as i32;
/// The greatest sprite number, which is its handle: sixteen bits, below
/// every control's handle.
pub const MAX_SPRITE_NUMBER: u16 = u16::MAX;
/// The bytes of one cell of a block, as operation `TT_SPRITE` takes it:
/// its kind, its code, its ink, its paper, and for a character its
/// triplet's mode and the designation of its sets.
pub const CELL_BYTES: usize = 6;
/// The bytes of one cell of a text, as operation `TT_BLOCK_TEXT` gives
/// it: its triplet's mode, its data and the designation of its sets.
pub const TEXT_BYTES: usize = 3;
/// The layer of a sprite that names none: the sprites' layer (ANIM-12).
pub const DEFAULT_SPRITE_LAYER: i32 = 2;
/// The greatest six-bit mosaic: bit 0 the top left sixel, bit 5 the
/// bottom right (EN 300 706 clause 15.6.1).
pub const LAST_SIXELS: u8 = 0x3F;
/// The ink of a block's cell that names none: white, which a Passive
/// object starts with (clause 13.5).
pub const DEFAULT_INK: u8 = neoteletext_core::consts::COLOUR_WHITE;
/// The paper of a block's cell that names none: black, likewise.
pub const DEFAULT_PAPER: u8 = neoteletext_core::consts::COLOUR_BLACK;
/// The most a sprite's making adds to one step's output: the objects'
/// section whole and every sprite's declaration.
pub const SPRITE_OUTPUT_BYTES: usize = OBJECT_PAGE_TRIPLETS * size_of::<protocol::script::batch::Triplet>()
    + MAX_TELETEXT_OBJECTS * size_of::<protocol::script::batch::TeletextObject>()
    + MAX_SPRITES * (size_of::<protocol::script::batch::Sprite>() + MAX_OBJECT_FRAMES * size_of::<u16>());
/// A character cell's designation byte when no Modified G0 and G2
/// Character Set Designation is to be written for it: a space, which
/// every set shows alike.
pub const NO_SETS: u8 = super::super::text::NO_PAGE_SET;

// A sprite's number is never a control's handle; a sprite of the tallest
// block and the widest fits the record; the batch's section holds every
// object of the page; the shares are the whole Object page; the labels a
// step may carry hold one for every thing; and one sprite's cells fit a
// host call.
const _: () = assert!((MAX_SPRITE_NUMBER as i32) < CONTROL_HANDLE_BASE);
const _: () = assert!(MAX_BLOCK_ROWS <= MAX_SPRITE_HEIGHT as i32 && MAX_BLOCK_COLUMNS <= MAX_SPRITE_WIDTH as i32);
const _: () = assert!(OBJECT_SUBPAGES * OBJECTS_PER_SUBPAGE <= MAX_TELETEXT_OBJECTS && MAX_OBJECT_BODY <= MAX_OBJECT_TRIPLETS);
const _: () = assert!(CHANNEL_SHARE_OBJECT_SUBPAGES + PAGE_SHARE_OBJECT_SUBPAGES == OBJECT_SUBPAGES && CHANNEL_SHARE_SPRITES < MAX_SPRITES);
const _: () = assert!(MAX_SPRITES + MAX_CONTROLS <= protocol::script::batch::MAX_LABELS);
const _: () = assert!(MAX_OBJECT_FRAMES * PLANE_ROWS * COLUMNS * CELL_BYTES <= crate::limits::MAX_HOST_BYTES);
const _: () = assert!(MAX_SPRITES + MAX_CONTROLS <= MAX_ANIMATE_OPERATIONS && LAST_FRAME as usize == MAX_OBJECT_FRAMES - 1);
// The engine's core and its rule table agree on the animation state's
// bounds, which the host's count of them rests on.
const _: () = assert!(LAST_LAYER == neoteletext_rules::limits::MAX_LAYER);
const _: () = assert!(FIRST_SPRITE_LAYER <= DEFAULT_SPRITE_LAYER && DEFAULT_SPRITE_LAYER <= LAST_SPRITE_LAYER && TEXT_BYTES < CELL_BYTES);
