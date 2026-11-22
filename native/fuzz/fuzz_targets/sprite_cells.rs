//! A sprite's frame as the script host compiles it into a Passive
//! object's body (script-api.md section 5.6; operation 112 TT_SPRITE;
//! `neoteletext_script::teletext::sprites::object::compile`): the cells
//! are a script's `Uint8Array`, six bytes each. Whatever the cells, the
//! body is within an object's bound and every triplet within its
//! fields; a triplet of the column address group writes no DRCS
//! character, and the only triplet of the row address group is Set
//! Active Position (EN 300 706 section 12.3.1, table 27), so that a
//! sprite can neither invoke an object nor end the page's enhancement
//! data nor reach a DRCS, which teletext pages of this browser never have.
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext_script::teletext::sprites::limits::{CELL_BYTES, MAX_BLOCK_COLUMNS, MAX_BLOCK_ROWS, OBJECT_BODY_TRIPLETS};
use neoteletext_script::teletext::sprites::object::compile;
use protocol::script::batch::{MAX_TRIPLET_ADDRESS, MAX_TRIPLET_DATA, MAX_TRIPLET_MODE};

/// The first address of the row address group: addresses 0 to 39 are
/// columns (EN 300 706 section 12.3.1).
const FIRST_ROW_ADDRESS: u8 = 40;
/// Set Active Position, of the row address group (table 27).
const SET_ACTIVE_POSITION: u8 = 0x04;
/// A DRCS character, of the column address group (table 27).
const DRCS_CHARACTER: u8 = 0x0D;

fuzz_target!(|data: &[u8]| {
    let Some((&[columns, rows], cells)) = data.split_first_chunk::<2>() else { return };
    let columns = columns % u8::try_from(MAX_BLOCK_COLUMNS).unwrap_or(u8::MAX) + 1;
    let rows = rows % u8::try_from(MAX_BLOCK_ROWS).unwrap_or(u8::MAX) + 1;
    // The caller has held the length: exactly a frame's cells, repeated
    // from the input to fill it.
    let wanted = usize::from(columns) * usize::from(rows) * CELL_BYTES;
    if cells.is_empty() {
        return;
    }
    let cells: Vec<u8> = cells.iter().copied().cycle().take(wanted).collect();
    let Ok(body) = compile(columns, rows, &cells) else { return };
    assert!(body.len() <= OBJECT_BODY_TRIPLETS, "a body of {} triplets", body.len());
    for triplet in &body {
        assert!(triplet.address <= MAX_TRIPLET_ADDRESS && triplet.mode <= MAX_TRIPLET_MODE && triplet.data <= MAX_TRIPLET_DATA, "{triplet:?}");
        if triplet.address < FIRST_ROW_ADDRESS {
            assert!(triplet.mode != DRCS_CHARACTER, "a sprite wrote a DRCS character: {triplet:?}");
        } else {
            assert_eq!(triplet.mode, SET_ACTIVE_POSITION, "a sprite wrote a row triplet other than Set Active Position: {triplet:?}");
        }
    }
});
