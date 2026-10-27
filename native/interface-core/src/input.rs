//! The viewer's input as the interface gives it to the trusted core
//! (`neoteletext-common/INPUT.md` section 3.3, REQ-INT-132): *intents*,
//! which the trusted core checks whole, applies the session's modes to and
//! alone encodes. The interface never writes the protocol.

use alloc::string::String;

use neoteletext_input::event::{self, Action, Event, Item, KeyCode, MouseAction, Point};

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
    /// A mouse button or move over the page.
    Mouse {
        /// Press, release or move.
        action: MouseAction,
        /// The button, or on a move the buttons held.
        button: u8,
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

impl Intent {
    /// The intent as the input stream has it, before the session's modes.
    #[must_use]
    pub fn item(&self) -> Item<'_> {
        Item::Event(match self {
            Self::Text(text) => return Item::Text(text),
            Self::Key { action, key, mods, text, base } => {
                Event::Key(event::Key { action: *action, key: *key, mods: *mods, text: text.as_deref(), base: *base })
            }
            Self::Mouse { action, button, at } => Event::Mouse(event::Mouse { action: *action, button: *button, at: *at }),
            Self::Wheel { dx, dy, at } => Event::Wheel(event::Wheel { dx: *dx, dy: *dy, at: *at }),
            Self::Paste(text) => Event::Paste(text),
            Self::Compose { cursor, text } => Event::Compose { cursor: *cursor, text },
            Self::Hover { leave, column, row } => Event::Hover { leave: *leave, column: u32::from(*column), row: u32::from(*row) },
        })
    }
}
