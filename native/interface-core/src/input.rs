//! The viewer's input as the interface gives it to the trusted core
//! (`interaction.md` section 7.1, REQ-INT-132): *intents*,
//! which the trusted core checks whole, paces and alone encodes. The
//! interface never writes the protocol: a move is each change of the dot
//! under the pointer, a touch is the system's own report of a contact,
//! and the trusted core keeps the pace, numbers the contacts and says when
//! the pointer has gone (DOC-1945, REQ-RCH-036, REQ-RCH-053).

use alloc::string::String;

use neoteletext_input::event::{self, Action, Device, Event, Item, KeyCode, MouseAction, Point, Stamp};
use neoteletext_input::keys;

/// The mouse's buttons as MOUSE numbers them on a press and a release
/// (DOC-1938); on a move, bit `n` is button `n` held.
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
        /// The modifier bits held, as DOC-1935 numbers them.
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
    /// The pointer entered or left a control of the page (DOC-1940): the control's first cell.
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
    /// Whether this is an act of the viewer's on the page, as the HTML
    /// Standard's activation triggering input event is
    /// (neoteletext-common's DEC-1684 item 6): a key pressed but Esc, a
    /// button pressed, a touch ended, text or a paste. A hover, a move, a
    /// release, the wheel and an unfinished composition are not.
    #[must_use]
    pub fn is_activation(&self) -> bool {
        match self {
            Self::Key { action, key, .. } => *action == Action::Press && !matches!(key, KeyCode::Functional(code) if keys::name(*code) == Some("Escape")),
            Self::Mouse { action, .. } => *action == MouseAction::Press,
            Self::Touch { phase, .. } => *phase == TouchPhase::Ended,
            Self::Text(_) | Self::Paste(_) => true,
            Self::Gone | Self::Wheel { .. } | Self::Compose { .. } | Self::Hover { .. } => false,
        }
    }

    /// The intent as the input stream has it, from the mouse, stamped with
    /// the page on screen (DOC-1934,
    /// REQ-1555); `None` for what only the trusted core can write, gone and
    /// a contact, which it numbers and places from its own state.
    #[must_use]
    pub fn item(&self, on: Stamp) -> Option<Item<'_>> {
        Some(Item::Event(match self {
            Self::Text(text) => return Some(Item::Text(text)),
            Self::Key { action, key, mods, text, base } => {
                Event::Key(event::Key { on, action: *action, key: *key, mods: *mods, text: text.as_deref(), base: *base, lost: false })
            }
            Self::Mouse { action, button, at } => Event::Mouse(event::Mouse { on, action: *action, button: *button, at: *at, device: Device::Mouse }),
            Self::Wheel { dx, dy, at } => Event::Wheel(event::Wheel { on, dx: *dx, dy: *dy, at: *at }),
            Self::Paste(text) => Event::Paste { on, text },
            Self::Compose { cursor, text } => Event::Compose { on, cursor: *cursor, text },
            Self::Hover { leave, column, row } => Event::Hover { on, leave: *leave, column: u32::from(*column), row: u32::from(*row) },
            Self::Gone | Self::Touch { .. } => return None,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(action: Action, key: KeyCode) -> Intent {
        Intent::Key { action, key, mods: 0, text: None, base: None }
    }

    // DEC-1684 item 6: what counts as the viewer's act on a page.
    #[test]
    fn only_a_viewers_act_counts() {
        let at = Point { column: 1, row: 1, pixels: None, mods: 0 };
        let escape = keys::code("Escape").map_or(KeyCode::Character(' '), KeyCode::Functional);
        assert!(matches!(escape, KeyCode::Functional(_)));
        let acts = [
            key(Action::Press, KeyCode::Character('a')),
            Intent::Mouse { action: MouseAction::Press, button: 0, at },
            Intent::Touch { phase: TouchPhase::Ended, contact: 1, at },
            Intent::Text("a".into()),
            Intent::Paste("a".into()),
        ];
        assert!(acts.iter().all(Intent::is_activation));
        let not_acts = [
            key(Action::Press, escape),
            key(Action::Repeat, KeyCode::Character('a')),
            key(Action::Release, KeyCode::Character('a')),
            Intent::Mouse { action: MouseAction::Release, button: 0, at },
            Intent::Mouse { action: MouseAction::Move, button: 0, at },
            Intent::Touch { phase: TouchPhase::Began, contact: 1, at },
            Intent::Touch { phase: TouchPhase::Moved, contact: 1, at },
            Intent::Touch { phase: TouchPhase::Cancelled, contact: 1, at },
            Intent::Wheel { dx: 0, dy: 120, at },
            Intent::Compose { cursor: 0, text: "a".into() },
            Intent::Hover { leave: false, column: 1, row: 1 },
            Intent::Gone,
        ];
        assert!(!not_acts.iter().any(Intent::is_activation));
    }
}
