//! The typing state on the wire (REQ-INT-131): whether keys go to the
//! open page's channel, and its cursor, checked by the
//! engine's own rules on each side.

use neoteletext_input::cursor;

use super::{CursorState, Typing};
use crate::{Fields, ProtocolError};

const FLAG_ON: u8 = 1;
const FLAG_SECURE: u8 = 2;
const FLAG_CURSOR: u8 = 4;
const FLAG_ROUND_TRIP: u8 = 8;
const FLAG_SHOW: u8 = 16;
const FLAG_REFUSED: u8 = 32;
const ALL_FLAGS: u8 = FLAG_ON | FLAG_SECURE | FLAG_CURSOR | FLAG_ROUND_TRIP | FLAG_SHOW | FLAG_REFUSED;

pub(crate) fn push_typing(payload: &mut Vec<u8>, typing: &Typing) -> Result<(), ProtocolError> {
    let flag = |set: bool, bit: u8| if set { bit } else { 0 };
    payload.extend_from_slice(&typing.channel.to_be_bytes());
    payload.extend_from_slice(&typing.page.to_be_bytes());
    payload.push(
        flag(typing.on, FLAG_ON)
            | flag(typing.secure, FLAG_SECURE)
            | flag(typing.cursor.is_some(), FLAG_CURSOR)
            | flag(typing.round_trip_ms.is_some(), FLAG_ROUND_TRIP)
            | flag(typing.show.is_some(), FLAG_SHOW)
            | flag(typing.refused, FLAG_REFUSED),
    );
    let place = typing.cursor.unwrap_or(CursorState { row: cursor::NO_CURSOR, column: 0, width: 0, flags: 0 });
    check_cursor(typing.page, typing.cursor)?;
    payload.extend_from_slice(&[place.row, place.column, place.width, place.flags]);
    payload.extend_from_slice(&typing.acked.to_be_bytes());
    payload.extend_from_slice(&typing.round_trip_ms.unwrap_or(0).to_be_bytes());
    if let Some(show) = typing.show {
        interface_core::channel::page_index(show).ok_or(ProtocolError::BadValue("page number"))?;
        payload.extend_from_slice(&show.to_be_bytes());
    }
    Ok(())
}

/// The cursor's fields are checked again on each side, by the engine's
/// own rules (REQ-INT-134).
fn check_cursor(page: u16, state: Option<CursorState>) -> Result<(), ProtocolError> {
    let Some(state) = state else { return Ok(()) };
    let [p0, p1] = page.to_be_bytes();
    cursor::from_body([p0, p1, 0, 0, state.row, state.column, state.width, state.flags])
        .ok()
        .filter(|checked| checked.place.is_some())
        .map(|_| ())
        .ok_or(ProtocolError::BadValue("cursor"))
}

pub(crate) fn read_typing(fields: &mut Fields<'_>) -> Result<Typing, ProtocolError> {
    let channel = fields.u32()?;
    let page = fields.display_page()?;
    let flags = fields.u8()?;
    if flags & !ALL_FLAGS != 0 {
        return Err(ProtocolError::BadValue("typing flags"));
    }
    let place = CursorState { row: fields.u8()?, column: fields.u8()?, width: fields.u8()?, flags: fields.u8()? };
    let cursor = (flags & FLAG_CURSOR != 0).then_some(place);
    check_cursor(page, cursor)?;
    let acked = fields.u32()?;
    let round_trip = fields.u16()?;
    let show = if flags & FLAG_SHOW != 0 { Some(fields.display_page()?) } else { None };
    Ok(Typing {
        channel,
        page,
        on: flags & FLAG_ON != 0,
        secure: flags & FLAG_SECURE != 0,
        refused: flags & FLAG_REFUSED != 0,
        cursor,
        acked,
        round_trip_ms: (flags & FLAG_ROUND_TRIP != 0).then_some(round_trip),
        show,
    })
}
