//! Typing into a page (`neoteletext-common/INPUT.md`, the input system):
//! the viewer's input as *intents* from the interface to the trusted core,
//! the typing state back, and the encoded input messages the trusted core
//! gives a fetcher. The interface never encodes the protocol
//! (REQ-INT-132): an intent is the browser's own message, which the
//! trusted core checks whole, applies the session's modes to and encodes.

mod intent;
#[cfg(test)]
mod tests;
mod typing;

use interface_core::canvas::PageKey;
use neoteletext_input::event;
use neoteletext_input::{MAX_COMPOSE, MAX_KEY_TEXT, MAX_MESSAGE, MAX_PASTE};

use crate::ProtocolError;

pub(crate) use intent::{push_input, read_input};
pub use interface_core::input::Intent;
pub(crate) use typing::{push_typing, read_typing};

// The pipe's kinds of typing (`interface/kinds.rs` lists every kind of
// the pipe): the interface's intents, then the trusted core's state.
pub(crate) const KIND_INPUT: u8 = 27;
pub(crate) const KIND_EDITING: u8 = 30;
pub(crate) const KIND_TYPING: u8 = 40;

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

/// An input message for the fetcher to write: whole items of the engine's
/// stream, read again by its reader, at most [`MAX_MESSAGE`] bytes
/// (REQ-INT-133).
pub(crate) fn check_message(bytes: &[u8]) -> Result<(), ProtocolError> {
    if bytes.len() > MAX_MESSAGE {
        return Err(ProtocolError::TooLong(bytes.len()));
    }
    event::read_message(bytes, &mut |_| {}).map_err(|_| ProtocolError::BadValue("input message"))
}
