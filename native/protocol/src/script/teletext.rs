//! The handle of a teletext control (the script API's specification,
//! sections 5.4 and 10.1 rule 4), in the one place the script host and the
//! decoder both read it.
//!
//! A control is declared by control triplets in the page's local
//! enhancement data (the engine's decision E.127), which carry no number:
//! the engine numbers a page's controls in the order their triplets come,
//! so a control's number changes when another is declared before it. Its
//! handle must not, so the handle names the control's first cell, which
//! no other control can hold. The script host gives it to the script; the
//! decoder makes the same handle from the control the pointer hit.

use neoteletext_core::consts::COLUMNS;

use super::batch::{FIRST_SCRIPT_ROW, LAST_SCRIPT_ROW};

/// The first handle of a control: above every sprite's, which is the
/// sprite's number of 16 bits (the engine's `SPRITES` record).
pub const CONTROL_HANDLE_BASE: i32 = 0x0001_0000;

/// The columns of a row, as a handle counts them.
const HANDLE_COLUMNS: i32 = {
    assert!(COLUMNS <= u8::MAX as usize);
    #[expect(clippy::cast_possible_truncation, clippy::cast_possible_wrap, reason = "the assertion above holds it to 8 bits")]
    let columns = COLUMNS as i32;
    columns
};

/// The handle of the control whose first cell is at `row`, `column`, or
/// none when that is no cell a script's control can start at.
#[must_use]
pub const fn control_handle(row: u8, column: u8) -> Option<i32> {
    if row < FIRST_SCRIPT_ROW || row > LAST_SCRIPT_ROW || column as i32 >= HANDLE_COLUMNS {
        return None;
    }
    Some(CONTROL_HANDLE_BASE.saturating_add((row as i32).saturating_mul(HANDLE_COLUMNS)).saturating_add(column as i32))
}

/// The first cell, row then column, of the control `handle` names, or
/// none when it is no control's handle.
#[must_use]
pub const fn control_cell(handle: i32) -> Option<(u8, u8)> {
    let Some(offset) = handle.checked_sub(CONTROL_HANDLE_BASE) else { return None };
    let (Some(row), Some(column)) = (offset.checked_div(HANDLE_COLUMNS), offset.checked_rem(HANDLE_COLUMNS)) else { return None };
    if offset < 0 || row < FIRST_SCRIPT_ROW as i32 || row > LAST_SCRIPT_ROW as i32 {
        return None;
    }
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "the row is held to 1 to 23 and the column to 0 to 39 just above")]
    let cell = (row as u8, column as u8);
    Some(cell)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Section 10.1 rule 4: every cell a control can start at has one
    /// handle, which names that cell and no other, and nothing else is a
    /// control's handle.
    #[test]
    fn a_controls_handle_names_its_first_cell() {
        let mut seen = Vec::new();
        for row in FIRST_SCRIPT_ROW..=LAST_SCRIPT_ROW {
            for column in 0..u8::try_from(COLUMNS).unwrap() {
                let handle = control_handle(row, column).unwrap();
                assert_eq!(control_cell(handle), Some((row, column)));
                assert!(handle >= CONTROL_HANDLE_BASE && !seen.contains(&handle));
                seen.push(handle);
            }
        }
        assert_eq!(control_handle(0, 0), None);
        assert_eq!(control_handle(LAST_SCRIPT_ROW + 1, 0), None);
        assert_eq!(control_handle(1, u8::try_from(COLUMNS).unwrap()), None);
        for handle in [0, -1, 1, CONTROL_HANDLE_BASE - 1, CONTROL_HANDLE_BASE, CONTROL_HANDLE_BASE + 39, i32::MAX, i32::MIN] {
            assert_eq!(control_cell(handle), None, "{handle}");
        }
    }
}
