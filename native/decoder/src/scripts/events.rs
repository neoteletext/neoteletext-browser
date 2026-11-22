//! The viewer's input as a script host is given it (the script API's
//! specification sections 4.2, 4.4 and 10.3): each event's kind, its four
//! whole numbers and its payload, packed as the script library reads
//! them (`neoteletext_script::ops`, which this process may not link; the
//! script-host program's tests hold the two equal).

use protocol::decoder::script::PointerKind;
use protocol::rules::{KeyAction, PointerAction};
use protocol::script::{Event, EventKind};

/// The bits of a key event's first number (the script library's
/// `KeyBit`).
pub const KEY_SHIFT: i32 = 1;
/// Control is held.
pub const KEY_CTRL: i32 = 2;
/// Alt is held.
pub const KEY_ALT: i32 = 4;
/// Meta is held.
pub const KEY_META: i32 = 8;
/// A key down that repeats, or a release that was not seen.
pub const KEY_REPEAT_OR_LOST: i32 = 16;

/// What a pointer event says the pointer did, in the low four bits of its
/// first number (the script library's `PointerAction`).
pub const POINTER_DOWN: i32 = 1;
/// It came up.
pub const POINTER_UP: i32 = 2;
/// It moved.
pub const POINTER_MOVE: i32 = 3;
/// It entered a thing.
pub const POINTER_ENTER: i32 = 4;
/// It left a thing.
pub const POINTER_LEAVE: i32 = 5;
/// The press was taken away.
pub const POINTER_CANCEL: i32 = 6;
/// Where the button is in a pointer event's first number.
pub const POINTER_BUTTON_SHIFT: u32 = 4;
/// Where the pointer's kind is in a pointer event's first number: two
/// bits, 0 the mouse, 1 a touch, 2 a pen (`PointerKind::code`).
pub const POINTER_KIND_SHIFT: u32 = 8;
/// The bit that says the pointer is the primary one of its kind.
pub const POINTER_PRIMARY: i32 = 1 << 10;
/// The mouse's number among the pointers (section 4.2's `pointerId`); a
/// contact of a touch screen has a later one.
pub const POINTER_ID: i32 = 1;

/// Which pointer an event is of (section 4.2: `pointerId`, `pointerType`
/// and `isPrimary`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Which {
    /// Its number, the same from its press to its release.
    pub id: i32,
    /// Its kind.
    pub kind: PointerKind,
    /// It is the primary pointer of its kind.
    pub primary: bool,
}

impl Which {
    /// The mouse: what a page's rule table input is of.
    pub const MOUSE: Self = Self { id: POINTER_ID, kind: PointerKind::Mouse, primary: true };

    /// The kind and the primary bit, as a pointer event's first number
    /// holds them.
    fn packed(self) -> i32 {
        let kind = i32::from(self.kind.code()).checked_shl(POINTER_KIND_SHIFT).unwrap_or(0);
        kind | if self.primary { POINTER_PRIMARY } else { 0 }
    }
}

// The engine's modifier bits, as the trusted core gives them with a key
// (`neoteletext_input::event`).
const MOD_SHIFT: u8 = 1;
const MOD_ALT: u8 = 2;
const MOD_CTRL: u8 = 4;
const MOD_SUPER: u8 = 8;
const MOD_META: u8 = 32;
#[cfg(feature = "interaction")]
const _: () = {
    use neoteletext_input::event;
    assert!(MOD_SHIFT == event::SHIFT && MOD_ALT == event::ALT && MOD_CTRL == event::CONTROL && MOD_SUPER == event::SUPER && MOD_META == event::META);
};

/// The W3C UI Events key value of a key the trusted core gave: a
/// character as itself, a named key by the engine's name for its code.
/// None for a code that names nothing, and for a named key in a build
/// without typing, which has no names.
#[must_use]
pub fn key_name(named: bool, code: u32) -> Option<String> {
    if named {
        #[cfg(feature = "interaction")]
        let name = neoteletext_input::keys::name(code);
        #[cfg(not(feature = "interaction"))]
        let name: Option<&'static str> = None;
        name.map(str::to_owned)
    } else {
        char::from_u32(code).filter(|character| !character.is_control()).map(|character| character.to_string())
    }
}

/// The modifiers of a key event's first number.
fn modifiers(mods: u8) -> i32 {
    let bit = |mask: u8, bit: i32| if mods & mask == 0 { 0 } else { bit };
    bit(MOD_SHIFT, KEY_SHIFT) | bit(MOD_CTRL, KEY_CTRL) | bit(MOD_ALT, KEY_ALT) | bit(MOD_SUPER | MOD_META, KEY_META)
}

/// A key's event (section 10.3, kinds 2 and 3).
#[must_use]
pub fn key(action: KeyAction, mods: u8, name: &str) -> Event {
    let (kind, repeat) = match action {
        KeyAction::Press => (EventKind::KeyDown, 0),
        KeyAction::Repeat => (EventKind::KeyDown, KEY_REPEAT_OR_LOST),
        KeyAction::Release => (EventKind::KeyUp, 0),
    };
    Event { kind, numbers: [modifiers(mods) | repeat, 0, 0, 0], payload: name.as_bytes().to_vec() }
}

/// The release of a key that was not seen (section 4.2: `lost`).
#[must_use]
pub fn lost_release(name: &str) -> Event {
    Event { kind: EventKind::KeyUp, numbers: [KEY_REPEAT_OR_LOST, 0, 0, 0], payload: name.as_bytes().to_vec() }
}

/// Whole numbers as an event's payload: four bytes each, big-endian.
fn ints(numbers: impl IntoIterator<Item = i32>) -> Vec<u8> {
    numbers.into_iter().flat_map(i32::to_be_bytes).collect()
}

/// The pointer's event (section 10.3, kind 5): what it did, where in the
/// renderer's own units (section 4.4: a teletext page's cell, else the
/// picture's pixel or dot), and the things under it by their handles,
/// topmost first; the first is the thing hit.
#[must_use]
pub fn pointer(action: PointerAction, button: u8, at: (u16, u16), teletext: bool, things: &[i32], which: Which) -> Event {
    let (x, y) = if teletext {
        let (column, row) = crate::engine::script::cell_at(at.0, at.1);
        (i32::from(column), i32::from(row))
    } else {
        (i32::from(at.0), i32::from(at.1))
    };
    // On a press or a release `button` is the button; on a move, the
    // buttons held (the rule table's input says which).
    let (what, pressed, held) = match action {
        PointerAction::Press => (POINTER_DOWN, i32::from(button), 1_i32.checked_shl(u32::from(button)).unwrap_or(0)),
        PointerAction::Release => (POINTER_UP, i32::from(button), 0),
        PointerAction::Move => (POINTER_MOVE, 0, i32::from(button)),
        PointerAction::Gone => (POINTER_CANCEL, 0, 0),
    };
    let packed = what | pressed.checked_shl(POINTER_BUTTON_SHIFT).unwrap_or(0) | which.packed();
    let (hit, others) = things.split_first().map_or((0, &[][..]), |(hit, others)| (*hit, others));
    // The place within the thing is not known to this process for a
    // drawing or an actor: the pointer's own place stands in.
    Event { kind: EventKind::Pointer, numbers: [packed, x, y, hit], payload: ints([which.id, held, x, y].into_iter().chain(others.iter().copied())) }
}

/// The pointer came onto a thing or went off it (section 4.2:
/// `pointerenter`, `pointerleave`), as the trusted core's picking of a
/// ReGIS drawing says.
#[must_use]
pub fn hover(leave: bool, thing: i32) -> Event {
    let what = if leave { POINTER_LEAVE } else { POINTER_ENTER };
    Event { kind: EventKind::Pointer, numbers: [what | POINTER_PRIMARY, 0, 0, thing], payload: ints([POINTER_ID, 0, 0, 0]) }
}
