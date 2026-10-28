//! The viewer's input as the interface gives it to the trusted core
//! (`neoteletext-common/INPUT.md` section 3.3, REQ-INT-132): *intents*,
//! which the trusted core checks whole, paces and alone encodes. The
//! interface never writes the protocol: a move is each change of the dot
//! under the pointer, a touch is the system's own report of a contact,
//! and the trusted core keeps the pace, numbers the contacts and says when
//! the pointer has gone (INPUT.md section 9.8, REQ-RCH-036, REQ-RCH-053).

use alloc::string::String;

use neoteletext_input::event::{self, Action, Device, Event, Item, KeyCode, MouseAction, Point};

/// The mouse's buttons as MOUSE numbers them on a press and a release
/// (INPUT.md section 5.4); on a move, bit `n` is button `n` held.
pub mod mouse_button {
    /// The main button.
    pub const LEFT: u8 = 0;
    /// The middle button, or a press of the wheel.
    pub const MIDDLE: u8 = 1;
    /// The secondary button.
    pub const RIGHT: u8 = 2;
    /// The thumb button that goes back.
    pub const BACK: u8 = 3;
    /// The thumb button that goes forward.
    pub const FORWARD: u8 = 4;
}

/// One thing the viewer did on the open page, for the trusted core to send
/// on if the page's channel takes it (REQ-INT-130).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// A key pressed, repeating or released.
    Key {
        /// Press, repeat or release.
        action: Action,
        /// The key: its unshifted character on the active layout, or its
        /// functional code.
        key: KeyCode,
        /// The modifier bits held, as INPUT.md section 5.5 numbers them.
        mods: u8,
        /// The text the press produced, at most 32 bytes.
        text: Option<String>,
        /// The United States layout's character of the same physical key.
        base: Option<char>,
    },
    /// Text an input method committed, or a phone typed.
    Text(String),
    /// A mouse button or move over the page: each change of the dot under
    /// the pointer is a move, buttons held or not (REQ-RCH-030), which the
    /// trusted core paces (REQ-RCH-036).
    Mouse {
        /// Press, release or move; never gone, which the trusted core
        /// alone writes ([`Intent::Gone`]).
        action: MouseAction,
        /// The button, or on a move the buttons held.
        button: u8,
        /// Where.
        at: Point,
    },
    /// The pointer left the open page: onto its border, the browser's own
    /// controls or out of the window (REQ-RCH-034). The trusted core sends
    /// the waiting move, then gone, from what it last sent.
    Gone,
    /// A contact on a touch screen, as the system reports it (REQ-RCH-041
    /// to REQ-RCH-046): the trusted core numbers it and decides whether it
    /// goes as MOUSE or TOUCH.
    Touch {
        /// Down, moved, lifted or cancelled.
        phase: TouchPhase,
        /// The system's own identifier of the contact (winit's
        /// `Touch::id`), for as long as it touches.
        contact: u64,
        /// Where.
        at: Point,
    },
    /// The wheel over the page.
    Wheel {
        /// Across, in 1/120ths of a line.
        dx: i32,
        /// Down, in 1/120ths of a line.
        dy: i32,
        /// Where.
        at: Point,
    },
    /// A paste, data and never keystrokes.
    Paste(String),
    /// An input method's preedit; empty ends composition.
    Compose {
        /// A code-point offset into the text.
        cursor: u32,
        /// The preedit.
        text: String,
    },
    /// The pointer entered or left a control of the page (INPUT.md section
    /// 5.4's HOVER): the control's first cell.
    Hover {
        /// It left, rather than entered.
        leave: bool,
        /// The control's first column.
        column: u8,
        /// The control's top row.
        row: u8,
    },
}

/// What the system reports of a contact on a touch screen (winit's
/// `TouchPhase`, the W3C Pointer Events' down, move, up and cancel).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchPhase {
    /// The contact began.
    Began,
    /// It moved.
    Moved,
    /// It lifted.
    Ended,
    /// The system took it for itself: it goes as gone, never as a lift
    /// (REQ-RCH-046).
    Cancelled,
}

impl Intent {
    /// The intent as the input stream has it, from the mouse; `None` for
    /// what only the trusted core can write, gone and a contact, which it
    /// numbers and places from its own state.
    #[must_use]
    pub fn item(&self) -> Option<Item<'_>> {
        Some(Item::Event(match self {
            Self::Text(text) => return Some(Item::Text(text)),
            Self::Key { action, key, mods, text, base } => {
                Event::Key(event::Key { action: *action, key: *key, mods: *mods, text: text.as_deref(), base: *base })
            }
            Self::Mouse { action, button, at } => Event::Mouse(event::Mouse { action: *action, button: *button, at: *at, device: Device::Mouse }),
            Self::Wheel { dx, dy, at } => Event::Wheel(event::Wheel { dx: *dx, dy: *dy, at: *at }),
            Self::Paste(text) => Event::Paste(text),
            Self::Compose { cursor, text } => Event::Compose { cursor: *cursor, text },
            Self::Hover { leave, column, row } => Event::Hover { leave: *leave, column: u32::from(*column), row: u32::from(*row) },
            Self::Gone | Self::Touch { .. } => return None,
        }))
    }
}
