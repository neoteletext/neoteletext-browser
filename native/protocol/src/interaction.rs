//! Typing into a page (`interaction.md`): the messages that carry the
//! viewer's keys from the interface to the trusted core and on to the
//! fetcher, and the cursor back from the decoder through the trusted core
//! to the interface. Keys travel as the engine's key records
//! (`neoteletext_input::key`), checked by its reader at every boundary.

use interface_core::canvas::PageKey;
use neoteletext_input::cursor;
use neoteletext_input::key::{self, Event, MAX_RECORD, Record};

use super::{Fields, ProtocolError};

/// A key the viewer pressed on the open page, for the trusted core to
/// send on if the page's channel takes input (requirement 18.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyIntent {
    /// The open page.
    pub key: PageKey,
    /// The interface's own number for the key, counting up: the trusted
    /// core says which it has seen acknowledged, for local echo.
    pub local: u32,
    /// The record, with sequence number 0: the trusted core numbers it.
    /// Only key down, key up, text and a click come from the interface.
    pub record: Vec<u8>,
}

/// Where the cursor is and how it echoes, checked by the trusted core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorState {
    /// 1 to 23.
    pub row: u8,
    /// 0 to 39.
    pub column: u8,
    /// How many cells echo may run from the column.
    pub width: u8,
    /// `neoteletext_input::cursor`'s flags: echo, masked, block.
    pub flags: u8,
}

/// Whether keys go to a channel, and its cursor (requirement 18.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Typing {
    /// The channel whose page is open.
    pub channel: u32,
    /// The open page.
    pub page: u16,
    /// Keys go to the channel now.
    pub on: bool,
    /// Its WebSocket is encrypted; "Not secure" shows otherwise.
    pub secure: bool,
    /// The cursor the channel placed on this page, if any.
    pub cursor: Option<CursorState>,
    /// The interface's number of the last key the channel acted on.
    pub acked: u32,
    /// The last round trip, key to acknowledgement, in milliseconds.
    pub round_trip_ms: Option<u16>,
    /// The channel's page now, when its cursor record names another of its
    /// pages than the one open: a videotex host goes to a page by sending
    /// it (`interaction.md` section 5.1), and the browser shows it.
    pub show: Option<u16>,
}

const FLAG_ON: u8 = 1;
const FLAG_SECURE: u8 = 2;
const FLAG_CURSOR: u8 = 4;
const FLAG_ROUND_TRIP: u8 = 8;
const FLAG_SHOW: u8 = 16;
const ALL_FLAGS: u8 = FLAG_ON | FLAG_SECURE | FLAG_CURSOR | FLAG_ROUND_TRIP | FLAG_SHOW;

/// The kinds of record the interface may send: never page changes, focus
/// or resumption, which the trusted core alone decides.
fn from_interface(record: &Record<'_>) -> bool {
    matches!(record.event, Event::Down(_) | Event::Up(_) | Event::Text(_) | Event::Click { .. })
}

pub(crate) fn push_key(payload: &mut Vec<u8>, intent: &KeyIntent) -> Result<(), ProtocolError> {
    let record = key::read(&intent.record).map_err(|_| ProtocolError::BadValue("key record"))?;
    if !from_interface(&record) || record.sequence != 0 {
        return Err(ProtocolError::BadValue("key record"));
    }
    super::push_key(payload, intent.key);
    payload.extend_from_slice(&intent.local.to_be_bytes());
    payload.extend_from_slice(&intent.record);
    Ok(())
}

pub(crate) fn read_key(fields: &mut Fields<'_>) -> Result<KeyIntent, ProtocolError> {
    let key = fields.key()?;
    let local = fields.u32()?;
    let bytes = fields.take(fields.bytes.len())?;
    let record = key::read(bytes).map_err(|_| ProtocolError::BadValue("key record"))?;
    if !from_interface(&record) || record.sequence != 0 {
        return Err(ProtocolError::BadValue("key record"));
    }
    Ok(KeyIntent { key, local, record: bytes.to_vec() })
}

pub(crate) fn push_typing(payload: &mut Vec<u8>, typing: &Typing) -> Result<(), ProtocolError> {
    let flag = |set: bool, bit: u8| if set { bit } else { 0 };
    payload.extend_from_slice(&typing.channel.to_be_bytes());
    payload.extend_from_slice(&typing.page.to_be_bytes());
    payload.push(
        flag(typing.on, FLAG_ON)
            | flag(typing.secure, FLAG_SECURE)
            | flag(typing.cursor.is_some(), FLAG_CURSOR)
            | flag(typing.round_trip_ms.is_some(), FLAG_ROUND_TRIP)
            | flag(typing.show.is_some(), FLAG_SHOW),
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
/// own rules (requirement 18.6).
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
    let page = fields.u16()?;
    if interface_core::channel::page_index(page).is_none() {
        return Err(ProtocolError::BadValue("page number"));
    }
    let flags = fields.u8()?;
    if flags & !ALL_FLAGS != 0 {
        return Err(ProtocolError::BadValue("typing flags"));
    }
    let place = CursorState { row: fields.u8()?, column: fields.u8()?, width: fields.u8()?, flags: fields.u8()? };
    let cursor = (flags & FLAG_CURSOR != 0).then_some(place);
    check_cursor(page, cursor)?;
    let acked = fields.u32()?;
    let round_trip = fields.u16()?;
    let show = if flags & FLAG_SHOW != 0 {
        let show = fields.u16()?;
        interface_core::channel::page_index(show).ok_or(ProtocolError::BadValue("page number"))?;
        Some(show)
    } else {
        None
    };
    Ok(Typing {
        channel,
        page,
        on: flags & FLAG_ON != 0,
        secure: flags & FLAG_SECURE != 0,
        cursor,
        acked,
        round_trip_ms: (flags & FLAG_ROUND_TRIP != 0).then_some(round_trip),
        show,
    })
}

/// A key record for the fetcher to send: checked again by the engine's
/// reader, and at most [`MAX_RECORD`] bytes (requirement 18.4).
pub(crate) fn check_record(bytes: &[u8]) -> Result<(), ProtocolError> {
    if bytes.len() > MAX_RECORD {
        return Err(ProtocolError::TooLong(bytes.len()));
    }
    key::read(bytes).map(|_| ()).map_err(|_| ProtocolError::BadValue("key record"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use neoteletext_input::key::Press;

    fn record(event: Event<'_>, sequence: u16) -> Vec<u8> {
        let mut out = [0; MAX_RECORD];
        let length = Record { sequence, clock_ms: 5, event }.write(&mut out).unwrap();
        out[..length].to_vec()
    }

    #[test]
    fn the_interface_sends_only_keys_text_and_clicks_unnumbered() {
        let key = PageKey { channel: 1, page: 0x100 };
        let down = Event::Down(Press { key: u16::from(b'a'), modifiers: 0, physical: 0 });
        let mut payload = Vec::new();
        let intent = KeyIntent { key, local: 9, record: record(down, 0) };
        push_key(&mut payload, &intent).unwrap();
        assert_eq!(read_key(&mut Fields { bytes: &payload }).unwrap(), intent);
        for refused in [record(Event::Page(None), 0), record(Event::Focus(true), 0), record(Event::Resume(3), 0), record(down, 4)] {
            assert!(push_key(&mut Vec::new(), &KeyIntent { key, local: 1, record: refused.clone() }).is_err());
            let mut payload = Vec::new();
            super::super::push_key(&mut payload, key);
            payload.extend_from_slice(&1_u32.to_be_bytes());
            payload.extend_from_slice(&refused);
            assert!(read_key(&mut Fields { bytes: &payload }).is_err());
        }
    }

    #[test]
    fn typing_round_trips_and_a_bad_cursor_is_refused() {
        let typing = Typing {
            channel: 3,
            page: 0x200,
            on: true,
            secure: false,
            cursor: Some(CursorState { row: 5, column: 6, width: 10, flags: cursor::ECHO }),
            acked: 40,
            round_trip_ms: Some(12),
            show: Some(0x201),
        };
        let mut payload = Vec::new();
        push_typing(&mut payload, &typing).unwrap();
        assert_eq!(read_typing(&mut Fields { bytes: &payload }).unwrap(), typing);
        let bad = Typing { cursor: Some(CursorState { row: 24, column: 0, width: 0, flags: 0 }), ..typing };
        assert!(push_typing(&mut Vec::new(), &bad).is_err());
    }
}
