//! The viewer's input as intents on the wire (INPUT.md, REQ-INT-132):
//! a key, text, the mouse, the wheel, a paste or a composition, each
//! checked against the engine's bounds both ways.

use neoteletext_input::event::{Action, KeyCode, MouseAction, Point};
use neoteletext_input::{MAX_COLUMNS, MAX_COMPOSE, MAX_KEY_TEXT, MAX_PASTE, MAX_ROWS};

use super::{InputIntent, Intent, check};
use crate::{Fields, ProtocolError};

/// The intents' tags.
pub(super) const TAG_KEY: u8 = 1;
const TAG_TEXT: u8 = 2;
const TAG_MOUSE: u8 = 3;
const TAG_WHEEL: u8 = 4;
const TAG_PASTE: u8 = 5;
const TAG_COMPOSE: u8 = 6;
const TAG_HOVER: u8 = 7;
/// A key's kind: a character or a functional code.
pub(super) const KEY_CHARACTER: u8 = 0;
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
        Intent::Hover { leave, column, row } => payload.extend_from_slice(&[TAG_HOVER, u8::from(*leave), *column, *row]),
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
        TAG_HOVER => {
            let leave = fields.flag("leave")?;
            Intent::Hover { leave, column: fields.u8()?, row: fields.u8()? }
        }
        _ => return Err(ProtocolError::BadValue("intent")),
    };
    check(&intent)?;
    Ok(intent)
}

pub(crate) fn push_input(payload: &mut Vec<u8>, input: &InputIntent) -> Result<(), ProtocolError> {
    check(&input.intent)?;
    crate::push_key(payload, input.key);
    payload.extend_from_slice(&input.local.to_be_bytes());
    push_intent(payload, &input.intent)
}

pub(crate) fn read_input(fields: &mut Fields<'_>) -> Result<InputIntent, ProtocolError> {
    let key = fields.key()?;
    let local = fields.u32()?;
    let intent = read_intent(fields)?;
    Ok(InputIntent { key, local, intent })
}
