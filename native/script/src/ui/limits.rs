//! The bounds of the component vocabulary `nt.ui` (script-api.md sections
//! 8 and 10.6), each defined once. They are the vocabulary's own, the
//! same on every renderer; what a renderer's drawing of a component costs
//! is that renderer's bound, refused by that renderer's name.

use neoteletext_core::consts::COLUMNS;
use protocol::script::batch::{FIRST_SCRIPT_ROW, LAST_SCRIPT_ROW};

use crate::limits::{MAX_HOST_BYTES, MAX_RESULT_BYTES};

/// The most components a container's host holds (section 10.6).
pub const MAX_COMPONENTS: usize = 64;
/// The most items of one component: a menu's items, a table's rows, a
/// paragraph's lines.
pub const MAX_UI_ITEMS: usize = 256;
/// The most bytes of the options one call gives, as JSON text.
pub const MAX_UI_OPTION_BYTES: usize = 16_384;
/// The most bytes of a component's `id`.
pub const MAX_UI_ID_BYTES: usize = 64;
/// The most bytes the host holds for every component's options together
/// (section 10.6's `MAX_UI_STATE_BYTES`).
pub const MAX_UI_STATE_BYTES: usize = 262_144;
/// The most items of every component together: what the budget report
/// gives as `uiNodes`.
pub const MAX_UI_NODES: usize = MAX_COMPONENTS * MAX_UI_ITEMS;
/// The most JSON values one call's options hold: every item with its
/// members, with room for the options about them.
pub const MAX_UI_OPTION_VALUES: usize = MAX_UI_ITEMS * 8;
/// The most columns of a table.
pub const MAX_TABLE_COLUMNS: usize = 8;
/// The most prompts of a frame: the four coloured keys.
pub const MAX_PROMPTS: usize = 4;
/// The longest a field's value may be asked to be, in characters.
pub const MAX_FIELD_CHARS: usize = 256;
/// The greatest magnitude of a slider's or a progress bar's numbers.
pub const MAX_UI_NUMBER: i64 = 1_000_000;

/// The first handle of a component: above every teletext control's, which
/// are above every sprite's and every ReGIS drawing's number.
pub const COMPONENT_HANDLE_BASE: i32 = 0x0002_0000;

/// The grid components are placed on, on every renderer: 40 columns by
/// the 23 rows a script draws on, rows 1 to 23 (section 8, "Placement").
pub const GRID_COLUMNS: i32 = {
    assert!(COLUMNS <= i8::MAX as usize);
    #[expect(clippy::cast_possible_truncation, clippy::cast_possible_wrap, reason = "the assertion above holds it to 7 bits")]
    let columns = COLUMNS as i32;
    columns
};
/// The grid's first row.
pub const GRID_FIRST_ROW: i32 = FIRST_SCRIPT_ROW as i32;
/// The grid's last row.
pub const GRID_LAST_ROW: i32 = LAST_SCRIPT_ROW as i32;

// A call's options fit a host call, and a component's value its result;
// a component's handle is no control's (the last control's is its base
// and at most 24 rows of 40 cells on).
const _: () = assert!(MAX_UI_OPTION_BYTES <= MAX_HOST_BYTES && MAX_UI_OPTION_BYTES <= MAX_RESULT_BYTES);
const _: () = assert!(COMPONENT_HANDLE_BASE > protocol::script::teletext::CONTROL_HANDLE_BASE + (GRID_LAST_ROW + 1) * GRID_COLUMNS);
const _: () = assert!(MAX_UI_STATE_BYTES >= MAX_UI_OPTION_BYTES * 2);
