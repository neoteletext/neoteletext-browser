//! Typing into a page (`neoteletext-common/INPUT.md`, the input system):
//! the viewer's input as *intents* from the interface to the trusted core,
//! the typing state back, and the encoded input messages the trusted core
//! gives a fetcher. The interface never encodes the protocol
//! (REQ-INT-132): an intent is the browser's own message, which the
//! trusted core checks whole, applies the session's modes to and encodes.

use interface_core::canvas::PageKey;
use neoteletext_input::cursor;
use neoteletext_input::event::{self, Action, KeyCode, MouseAction, Point};
use neoteletext_input::{MAX_COLUMNS, MAX_COMPOSE, MAX_KEY_TEXT, MAX_MESSAGE, MAX_PASTE, MAX_ROWS};

use super::{Fields, ProtocolError};

pub use interface_core::input::Intent;

/// Whether an intent is within the protocol's bounds: what the engine's
/// writer writes and its reader reads back (REQ-INT-102, REQ-INT-103).
fn check(intent: &Intent) -> Result<(), ProtocolError> {
    let longest = match intent {
        Intent::Text(text) | Intent::Paste(text) | Intent::Compose { text, .. } => text.len(),
        Intent::Key { text, .. } => text.as_ref().map_or(0, String::len),
        Intent::Mouse { .. } | Intent::Wheel { .. } => 0,
    };
    let bound = match intent {
        Intent::Text(_) | Intent::Paste(_) => MAX_PASTE,
        Intent::Key { .. } => MAX_KEY_TEXT,
        Intent::Compose { .. } => MAX_COMPOSE,
        Intent::Mouse { .. } | Intent::Wheel { .. } => 0,
    };
    if longest > bound || matches!(intent, Intent::Text(text) if text.is_empty()) {
        return Err(ProtocolError::BadValue("intent"));
    }
    let mut out = [0_u8; MAX_MESSAGE];
    event::write(&intent.item(), &mut out).map(|_| ()).map_err(|_| ProtocolError::BadValue("intent"))
}

/// An intent for the open page, numbered by the interface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputIntent {
    /// The open page.
    pub key: PageKey,
    /// The interface's own number for it, counting up: the trusted core
    /// says which it has seen acknowledged, for local echo.
    pub local: u32,
    /// What the viewer did.
    pub intent: Intent,
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

/// Whether keys go to a channel, and its cursor (REQ-INT-131).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Typing {
    /// The channel whose page is open.
    pub channel: u32,
    /// The open page.
    pub page: u16,
    /// Keys go to the channel now: its session was welcomed.
    pub on: bool,
    /// Its WebSocket is encrypted; "Not secure" shows otherwise.
    pub secure: bool,
    /// The channel did not answer HELLO in time, or refused: it takes no
    /// input, and the strip says so.
    pub refused: bool,
    /// The input modes in force on the open page (INPUT.md section 6.3):
    /// whether clicks, the wheel, releases and composition go.
    pub modes: u32,
    /// The cursor the channel placed on this page, if any.
    pub cursor: Option<CursorState>,
    /// The interface's number of the last intent the channel acted on.
    pub acked: u32,
    /// The last round trip, input to acknowledgement, in milliseconds.
    pub round_trip_ms: Option<u16>,
    /// The channel's page now, when its cursor record names another of its
    /// pages than the one open: a host goes to a page by sending it
    /// (REQ-INT-113), and the browser shows it.
    pub show: Option<u16>,
}

const FLAG_ON: u8 = 1;
const FLAG_SECURE: u8 = 2;
const FLAG_CURSOR: u8 = 4;
const FLAG_ROUND_TRIP: u8 = 8;
const FLAG_SHOW: u8 = 16;
const FLAG_REFUSED: u8 = 32;
const ALL_FLAGS: u8 = FLAG_ON | FLAG_SECURE | FLAG_CURSOR | FLAG_ROUND_TRIP | FLAG_SHOW | FLAG_REFUSED;

/// The intents' tags.
const TAG_KEY: u8 = 1;
const TAG_TEXT: u8 = 2;
const TAG_MOUSE: u8 = 3;
const TAG_WHEEL: u8 = 4;
const TAG_PASTE: u8 = 5;
const TAG_COMPOSE: u8 = 6;
/// A key's kind: a character or a functional code.
const KEY_CHARACTER: u8 = 0;
const KEY_FUNCTIONAL: u8 = 1;
/// A field that is absent.
const ABSENT: u8 = 0;
const PRESENT: u8 = 1;
/// Character values outside Unicode's scalar values are refused.
const NO_CHARACTER: u32 = u32::MAX;

fn action_byte(action: Action) -> u8 {
    match action {
        Action::Press => 0,
        Action::Repeat => 1,
        Action::Release => 2,
    }
}

fn read_action(byte: u8) -> Result<Action, ProtocolError> {
    Ok(match byte {
        0 => Action::Press,
        1 => Action::Repeat,
        2 => Action::Release,
        _ => return Err(ProtocolError::BadValue("key action")),
    })
}

fn mouse_byte(action: MouseAction) -> u8 {
    match action {
        MouseAction::Press => 0,
        MouseAction::Release => 1,
        MouseAction::Move => 2,
    }
}

fn read_mouse(byte: u8) -> Result<MouseAction, ProtocolError> {
    Ok(match byte {
        0 => MouseAction::Press,
        1 => MouseAction::Release,
        2 => MouseAction::Move,
        _ => return Err(ProtocolError::BadValue("mouse action")),
    })
}

/// A string of any characters, a paste's newlines included: its length,
/// then its bytes.
fn push_string(payload: &mut Vec<u8>, text: &str) -> Result<(), ProtocolError> {
    let length = u16::try_from(text.len()).map_err(|_| ProtocolError::TooLong(text.len()))?;
    payload.extend_from_slice(&length.to_be_bytes());
    payload.extend_from_slice(text.as_bytes());
    Ok(())
}

fn read_string(fields: &mut Fields<'_>, limit: usize) -> Result<String, ProtocolError> {
    let length = usize::from(fields.u16()?);
    if length > limit {
        return Err(ProtocolError::TooLong(length));
    }
    let bytes = fields.take(length)?;
    std::str::from_utf8(bytes).map(str::to_owned).map_err(|_| ProtocolError::BadValue("text not UTF-8"))
}

fn push_point(payload: &mut Vec<u8>, at: Point) {
    payload.extend_from_slice(&at.column.to_be_bytes());
    payload.extend_from_slice(&at.row.to_be_bytes());
    match at.pixels {
        Some((x, y)) => {
            payload.push(PRESENT);
            payload.extend_from_slice(&x.to_be_bytes());
            payload.extend_from_slice(&y.to_be_bytes());
        }
        None => payload.push(ABSENT),
    }
    payload.push(at.mods);
}

fn read_point(fields: &mut Fields<'_>) -> Result<Point, ProtocolError> {
    let column = fields.u32()?;
    let row = fields.u32()?;
    if column >= MAX_COLUMNS || row >= MAX_ROWS {
        return Err(ProtocolError::BadValue("cell"));
    }
    let pixels = match fields.u8()? {
        ABSENT => None,
        PRESENT => Some((fields.u32()?, fields.u32()?)),
        _ => return Err(ProtocolError::BadValue("pixels")),
    };
    Ok(Point { column, row, pixels, mods: fields.u8()? })
}

fn push_intent(payload: &mut Vec<u8>, intent: &Intent) -> Result<(), ProtocolError> {
    match intent {
        Intent::Key { action, key, mods, text, base } => {
            payload.extend_from_slice(&[TAG_KEY, action_byte(*action)]);
            let (kind, value) = match key {
                KeyCode::Character(character) => (KEY_CHARACTER, u32::from(*character)),
                KeyCode::Functional(code) => (KEY_FUNCTIONAL, *code),
            };
            payload.push(kind);
            payload.extend_from_slice(&value.to_be_bytes());
            payload.push(*mods);
            match text {
                Some(text) => {
                    payload.push(PRESENT);
                    push_string(payload, text)?;
                }
                None => payload.push(ABSENT),
            }
            payload.extend_from_slice(&base.map_or(NO_CHARACTER, u32::from).to_be_bytes());
        }
        Intent::Text(text) => {
            payload.push(TAG_TEXT);
            push_string(payload, text)?;
        }
        Intent::Mouse { action, button, at } => {
            payload.extend_from_slice(&[TAG_MOUSE, mouse_byte(*action), *button]);
            push_point(payload, *at);
        }
        Intent::Wheel { dx, dy, at } => {
            payload.push(TAG_WHEEL);
            payload.extend_from_slice(&dx.to_be_bytes());
            payload.extend_from_slice(&dy.to_be_bytes());
            push_point(payload, *at);
        }
        Intent::Paste(text) => {
            payload.push(TAG_PASTE);
            push_string(payload, text)?;
        }
        Intent::Compose { cursor, text } => {
            payload.push(TAG_COMPOSE);
            payload.extend_from_slice(&cursor.to_be_bytes());
            push_string(payload, text)?;
        }
    }
    Ok(())
}

fn character(value: u32) -> Result<char, ProtocolError> {
    char::from_u32(value).ok_or(ProtocolError::BadValue("character"))
}

fn read_intent(fields: &mut Fields<'_>) -> Result<Intent, ProtocolError> {
    let intent = match fields.u8()? {
        TAG_KEY => {
            let action = read_action(fields.u8()?)?;
            let key = match (fields.u8()?, fields.u32()?) {
                (KEY_CHARACTER, value) => KeyCode::Character(character(value)?),
                (KEY_FUNCTIONAL, code) => KeyCode::Functional(code),
                _ => return Err(ProtocolError::BadValue("key kind")),
            };
            let mods = fields.u8()?;
            let text = match fields.u8()? {
                ABSENT => None,
                PRESENT => Some(read_string(fields, MAX_KEY_TEXT)?),
                _ => return Err(ProtocolError::BadValue("key text")),
            };
            let base = match fields.u32()? {
                NO_CHARACTER => None,
                value => Some(character(value)?),
            };
            Intent::Key { action, key, mods, text, base }
        }
        TAG_TEXT => Intent::Text(read_string(fields, MAX_PASTE)?),
        TAG_MOUSE => {
            let action = read_mouse(fields.u8()?)?;
            let button = fields.u8()?;
            Intent::Mouse { action, button, at: read_point(fields)? }
        }
        TAG_WHEEL => {
            let dx = i32::from_be_bytes(fields.array()?);
            let dy = i32::from_be_bytes(fields.array()?);
            Intent::Wheel { dx, dy, at: read_point(fields)? }
        }
        TAG_PASTE => Intent::Paste(read_string(fields, MAX_PASTE)?),
        TAG_COMPOSE => {
            let cursor = fields.u32()?;
            Intent::Compose { cursor, text: read_string(fields, MAX_COMPOSE)? }
        }
        _ => return Err(ProtocolError::BadValue("intent")),
    };
    check(&intent)?;
    Ok(intent)
}

pub(crate) fn push_input(payload: &mut Vec<u8>, input: &InputIntent) -> Result<(), ProtocolError> {
    check(&input.intent)?;
    super::push_key(payload, input.key);
    payload.extend_from_slice(&input.local.to_be_bytes());
    push_intent(payload, &input.intent)
}

pub(crate) fn read_input(fields: &mut Fields<'_>) -> Result<InputIntent, ProtocolError> {
    let key = fields.key()?;
    let local = fields.u32()?;
    let intent = read_intent(fields)?;
    Ok(InputIntent { key, local, intent })
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
            | flag(typing.show.is_some(), FLAG_SHOW)
            | flag(typing.refused, FLAG_REFUSED),
    );
    if typing.modes & !neoteletext_input::modes::ALL_MODES != 0 {
        return Err(ProtocolError::BadValue("modes"));
    }
    payload.extend_from_slice(&typing.modes.to_be_bytes());
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
    let page = fields.u16()?;
    if interface_core::channel::page_index(page).is_none() {
        return Err(ProtocolError::BadValue("page number"));
    }
    let flags = fields.u8()?;
    if flags & !ALL_FLAGS != 0 {
        return Err(ProtocolError::BadValue("typing flags"));
    }
    let modes = fields.u32()?;
    if modes & !neoteletext_input::modes::ALL_MODES != 0 {
        return Err(ProtocolError::BadValue("modes"));
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
        refused: flags & FLAG_REFUSED != 0,
        modes,
        cursor,
        acked,
        round_trip_ms: (flags & FLAG_ROUND_TRIP != 0).then_some(round_trip),
        show,
    })
}

/// An input message for the fetcher to write: whole items of the engine's
/// stream, read again by its reader, at most [`MAX_MESSAGE`] bytes
/// (REQ-INT-133).
pub(crate) fn check_message(bytes: &[u8]) -> Result<(), ProtocolError> {
    if bytes.len() > MAX_MESSAGE {
        return Err(ProtocolError::TooLong(bytes.len()));
    }
    event::read_message(bytes, &mut |_| {}).map_err(|_| ProtocolError::BadValue("input message"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use neoteletext_input::event::{ALT, CONTROL};

    const KEY: PageKey = PageKey { channel: 1, page: 0x100 };

    fn round_trip(intent: Intent) -> Result<InputIntent, ProtocolError> {
        let input = InputIntent { key: KEY, local: 9, intent };
        let mut payload = Vec::new();
        push_input(&mut payload, &input)?;
        let back = read_input(&mut Fields { bytes: &payload })?;
        assert_eq!(back, input);
        Ok(back)
    }

    fn at(column: u32, row: u32) -> Point {
        Point { column, row, pixels: Some((120, 100)), mods: CONTROL }
    }

    /// REQ-INT-132: the interface sends intents, each read whole.
    #[test]
    fn every_intent_round_trips() {
        for intent in [
            Intent::Key { action: Action::Press, key: KeyCode::Character('a'), mods: 0, text: Some("a".into()), base: Some('q') },
            Intent::Key { action: Action::Release, key: KeyCode::Functional(2), mods: CONTROL | ALT, text: None, base: None },
            Intent::Text("Grüße, мир".into()),
            Intent::Mouse { action: MouseAction::Press, button: 0, at: at(10, 5) },
            Intent::Mouse { action: MouseAction::Move, button: 1, at: Point { pixels: None, ..at(71, 24) } },
            Intent::Wheel { dx: 0, dy: -120, at: at(0, 0) },
            Intent::Paste("line one\nline two\t".into()),
            Intent::Compose { cursor: 1, text: "にほ".into() },
            Intent::Compose { cursor: 0, text: String::new() },
        ] {
            round_trip(intent).unwrap();
        }
    }

    #[test]
    fn what_breaks_a_bound_is_refused() {
        for refused in [
            Intent::Text(String::new()),
            Intent::Text("\u{7}".into()),
            Intent::Text("a".repeat(MAX_PASTE + 1)),
            Intent::Key { action: Action::Press, key: KeyCode::Functional(56), mods: 0, text: None, base: None },
            Intent::Key { action: Action::Press, key: KeyCode::Character('a'), mods: 0, text: Some("a".repeat(33)), base: None },
            Intent::Mouse { action: MouseAction::Press, button: 0, at: at(72, 0) },
            Intent::Mouse { action: MouseAction::Press, button: 0, at: at(0, 25) },
            Intent::Compose { cursor: 0, text: "a".repeat(MAX_COMPOSE + 1) },
            Intent::Paste("a".repeat(MAX_PASTE + 1)),
        ] {
            assert!(round_trip(refused.clone()).is_err(), "{refused:?}");
        }
        // Bytes the interface cannot send: an unknown tag, a bad character.
        let mut payload = Vec::new();
        super::super::push_key(&mut payload, KEY);
        payload.extend_from_slice(&1_u32.to_be_bytes());
        for tail in [vec![9_u8], vec![TAG_KEY, 0, KEY_CHARACTER, 0, 0, 0xD8, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]] {
            let mut bytes = payload.clone();
            bytes.extend_from_slice(&tail);
            assert!(read_input(&mut Fields { bytes: &bytes }).is_err(), "{tail:?}");
        }
    }

    #[test]
    fn editing_round_trips_and_only_as_a_flag() {
        use super::super::{ToCore, read_from_interface, write_to_core};
        for editing in [true, false] {
            let mut bytes = Vec::new();
            write_to_core(&mut bytes, &ToCore::Editing(editing)).unwrap();
            assert_eq!(read_from_interface(&mut bytes.as_slice()).unwrap().unwrap(), ToCore::Editing(editing));
            // The framing: the kind, a 32-bit length, the flag.
            bytes[5] = 2;
            assert!(read_from_interface(&mut bytes.as_slice()).is_err(), "neither false nor true");
        }
    }

    #[test]
    fn typing_round_trips_and_a_bad_cursor_or_mode_is_refused() {
        let typing = Typing {
            channel: 3,
            page: 0x200,
            on: true,
            secure: false,
            refused: false,
            modes: neoteletext_input::modes::MOUSE_BUTTONS,
            cursor: Some(CursorState { row: 5, column: 6, width: 10, flags: cursor::ECHO }),
            acked: 40,
            round_trip_ms: Some(12),
            show: Some(0x201),
        };
        let mut payload = Vec::new();
        push_typing(&mut payload, &typing).unwrap();
        assert_eq!(read_typing(&mut Fields { bytes: &payload }).unwrap(), typing);
        let refused = Typing { on: false, refused: true, cursor: None, show: None, ..typing.clone() };
        let mut payload = Vec::new();
        push_typing(&mut payload, &refused).unwrap();
        assert_eq!(read_typing(&mut Fields { bytes: &payload }).unwrap(), refused);
        let bad = Typing { cursor: Some(CursorState { row: 24, column: 0, width: 0, flags: 0 }), ..typing.clone() };
        assert!(push_typing(&mut Vec::new(), &bad).is_err());
        assert!(push_typing(&mut Vec::new(), &Typing { modes: 256, ..typing }).is_err());
    }

    /// REQ-INT-133: the fetcher takes whole items of the stream, at most
    /// 4 096 bytes.
    #[test]
    fn a_message_for_the_fetcher_is_whole_and_bounded() {
        assert!(check_message(b"abc\xFF\x01\x04\x02").is_ok());
        assert!(check_message(b"\xFF\x01").is_err(), "an item cut short");
        assert!(check_message(b"\xFF\x00").is_err(), "a reserved type");
        assert!(check_message(&[b'a'; MAX_MESSAGE + 1]).is_err());
    }
}
