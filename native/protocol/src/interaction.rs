//! Typing into a page (`neoteletext-common/INPUT.md`, the input system):
//! the viewer's input as *intents* from the interface to the trusted core,
//! the typing state back, and the encoded input messages the trusted core
//! gives a fetcher. The interface never encodes the protocol
//! (REQ-INT-132): an intent is the browser's own message, which the
//! trusted core checks whole and encodes.

mod intent;
#[cfg(test)]
mod tests;
mod typing;

use interface_core::canvas::PageKey;
use neoteletext_core::consts::{CELL_HEIGHT, CELL_WIDTH, PAGE_HEIGHT, PAGE_WIDTH};
use neoteletext_input::event::{self, Device, Item, Mouse, MouseAction, Point};
use neoteletext_input::{MAX_COMPOSE, MAX_KEY_TEXT, MAX_MESSAGE, MAX_PASTE};

use crate::ProtocolError;

pub(crate) use intent::{push_input, read_input};
pub use interface_core::input::{Intent, TouchPhase};
pub(crate) use typing::{push_typing, read_typing};

// The pipe's kinds of typing (`interface/kinds.rs` lists every kind of
// the pipe): the interface's intents, then the trusted core's state.
pub(crate) const KIND_INPUT: u8 = 27;
pub(crate) const KIND_EDITING: u8 = 30;
pub(crate) const KIND_TYPING: u8 = 40;
pub(crate) const KIND_INPUT_TRACE: u8 = 94;

pub use interface_core::trace::{MAX_TRACE, TraceLine};

/// A trace line, its text cleaned and within [`MAX_TRACE`].
pub(crate) fn push_trace(payload: &mut Vec<u8>, line: &TraceLine) -> Result<(), ProtocolError> {
    crate::push_text(payload, &line.text, MAX_TRACE)?;
    payload.extend_from_slice(&line.repeats.to_be_bytes());
    payload.extend_from_slice(&line.sent.to_be_bytes());
    Ok(())
}

pub(crate) fn read_trace(fields: &mut crate::Fields<'_>) -> Result<TraceLine, ProtocolError> {
    let text = fields.text(MAX_TRACE)?;
    let repeats = fields.u32()?;
    let sent = u64::from_be_bytes(fields.array()?);
    Ok(TraceLine { text, repeats, sent })
}

/// Whether the dot of a position, when it has one, lies in the page image
/// and inside the position's cell (INPUT.md section 9.9, REQ-INT-127): the
/// image is the engine's 480 by 500 dots, 12 by 20 to a cell, so `px` / 12
/// is the column and `py` / 20 the row. The interface is not trusted to say
/// where the pointer was, so the trusted core's reader refuses an intent
/// whose dot no pointer over that cell could have had.
#[must_use]
pub fn dot_in_cell(at: &Point) -> bool {
    let Some((x, y)) = at.pixels else { return true };
    let (Ok(x), Ok(y), Ok(column), Ok(row)) = (usize::try_from(x), usize::try_from(y), usize::try_from(at.column), usize::try_from(at.row)) else {
        return false;
    };
    x < PAGE_WIDTH && y < PAGE_HEIGHT && x.checked_div(CELL_WIDTH) == Some(column) && y.checked_div(CELL_HEIGHT) == Some(row)
}

/// Whether an intent is within the protocol's bounds: what the engine's
/// writer writes and its reader reads back (REQ-INT-102, REQ-INT-103), a
/// dot inside its cell (REQ-INT-127), and no gone, which only the trusted
/// core writes.
fn check(intent: &Intent) -> Result<(), ProtocolError> {
    let longest = match intent {
        Intent::Text(text) | Intent::Paste(text) | Intent::Compose { text, .. } => text.len(),
        Intent::Key { text, .. } => text.as_ref().map_or(0, String::len),
        Intent::Mouse { .. } | Intent::Wheel { .. } | Intent::Hover { .. } | Intent::Gone | Intent::Touch { .. } => 0,
    };
    let bound = match intent {
        Intent::Text(_) | Intent::Paste(_) => MAX_PASTE,
        Intent::Key { .. } => MAX_KEY_TEXT,
        Intent::Compose { .. } => MAX_COMPOSE,
        Intent::Mouse { .. } | Intent::Wheel { .. } | Intent::Hover { .. } | Intent::Gone | Intent::Touch { .. } => 0,
    };
    if longest > bound || matches!(intent, Intent::Text(text) if text.is_empty()) || matches!(intent, Intent::Mouse { action: MouseAction::Gone, .. }) {
        return Err(ProtocolError::BadValue("intent"));
    }
    let point = match intent {
        Intent::Mouse { at, .. } | Intent::Wheel { at, .. } | Intent::Touch { at, .. } => Some(at),
        _ => None,
    };
    if point.is_some_and(|at| !dot_in_cell(at)) {
        return Err(ProtocolError::BadValue("dot"));
    }
    // A contact's position is bounded as a press there is; gone has none.
    let item = match (intent.item(), intent) {
        (Some(item), _) => item,
        (None, Intent::Touch { at, .. }) => Item::Event(event::Event::Mouse(Mouse { action: MouseAction::Press, button: 0, at: *at, device: Device::Mouse })),
        (None, _) => return Ok(()),
    };
    let mut out = [0_u8; MAX_MESSAGE];
    event::write(&item, &mut out).map(|_| ()).map_err(|_| ProtocolError::BadValue("intent"))
}

/// An intent for the open page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputIntent {
    /// The open page.
    pub key: PageKey,
    /// What the viewer did.
    pub intent: Intent,
}

/// Where the cursor is and how it is drawn, checked by the trusted core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorState {
    /// 1 to 23.
    pub row: u8,
    /// 0 to 39.
    pub column: u8,
    /// `neoteletext_input::cursor::BLOCK` or none: the browser echoes
    /// nothing, so it is told no other flag.
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
    /// The cursor the channel placed on this page, if any.
    pub cursor: Option<CursorState>,
    /// The push connection's last round trip, WebSocket ping to pong, in
    /// microseconds (REQ-RCH-016), while keys go to the channel.
    pub round_trip_us: Option<u32>,
    /// The channel's page now, when its cursor record names another of its
    /// pages than the one open: a host goes to a page by sending it
    /// (REQ-INT-113), and the browser shows it.
    pub show: Option<u16>,
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

/// A message marked as `contact`'s move is that move alone: one item, a
/// MOUSE move for contact 0 or a TOUCH move of that `id` (the engine's
/// `event::move_contact`), so a fetcher holds only moves in a contact's
/// slot (REQ-RCH-035).
pub(crate) fn check_move(bytes: &[u8], contact: u8) -> Result<(), ProtocolError> {
    let mut items = 0_usize;
    let mut of_contact = true;
    event::read_message(bytes, &mut |item| {
        items = items.saturating_add(1);
        of_contact &= matches!(item, Item::Event(event) if event::move_contact(&event) == Some(usize::from(contact)));
    })
    .map_err(|_| ProtocolError::BadValue("input message"))?;
    if items == 1 && of_contact && usize::from(contact) < neoteletext_input::MAX_CONTACTS { Ok(()) } else { Err(ProtocolError::BadValue("move")) }
}
