//! Viewer state as the interface shows it (neoteletext-common's
//! `docs/viewer-state.md`, VS-109, VS-112 and VS-114, DEC-6471): the
//! settings' list "Channels that know you", which names each channel with
//! a key and says whether it is remembered, when it was made and last
//! used, and never the key or any part of it (VS-116); the requests the
//! viewer's controls make, by the numbers the view gave, so that the
//! interface can ask for nothing but changes to rows it was shown; and the
//! strip's mark of the active tab.

use alloc::string::String;
use alloc::vec::Vec;

pub use crate::library::{MAX_NOTICE, MAX_QUERY, MAX_VIEW_ROWS};

/// The longest label: a channel's identity, cut as a view's address is. A
/// row is acted on by its number, never by this text.
pub const MAX_LABEL: usize = crate::library::MAX_SHOWN_ADDRESS;
/// The longest "made" or "last used" text, as "4 days ago".
pub const MAX_WHEN: usize = 32;

/// One channel of the list: one with a key in the ordinary jar, or one the
/// viewer blocked (VS-112).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KnownRow {
    /// The number it is asked for by.
    pub number: u32,
    /// The channel's identity, as shown.
    pub label: String,
    /// The channel has a key; a blocked channel has none.
    pub has_key: bool,
    /// Its key is remembered, not a visit key.
    pub remembered: bool,
    /// The viewer blocked the channel: every later request to know the
    /// viewer is declined.
    pub blocked: bool,
    /// When its key was made, in words; empty without one.
    pub created: String,
    /// When its key was last used, in words; empty without one.
    pub used: String,
}

/// What the list shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KnownView {
    /// The search the rows answer; empty for all.
    pub query: String,
    /// The place of the first row among all that answer.
    pub first: u32,
    /// How many rows answer.
    pub total: u32,
    /// At most [`MAX_VIEW_ROWS`].
    pub rows: Vec<KnownRow>,
    /// Remembering is possible: the key file's sealing key can be had
    /// (VS-108).
    pub can_remember: bool,
    /// What the list says: why remembering is unavailable, the result of a
    /// change, or that a copied profile's keys were not carried (VS-118).
    pub notice: String,
}

/// What the viewer's controls ask, as strip input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KnownRequest {
    /// The list opened or closed: the trusted core sends views while open.
    Panel(bool),
    /// The rows that answer `text`, from the `first`.
    Query {
        /// The search; empty for all.
        text: String,
        /// The first row wanted.
        first: u32,
    },
    /// Forget a row's channel: its key is deleted.
    Forget(u32),
    /// Remember a row's channel, or stop remembering it.
    Remember {
        /// The channel's row.
        number: u32,
        /// Remembered.
        remembered: bool,
    },
    /// Block a row's channel, forgetting it, or unblock it.
    Block {
        /// The channel's row.
        number: u32,
        /// Blocked.
        blocked: bool,
    },
    /// Forget every channel.
    ForgetAll,
    /// "Remember me on this channel" for the active tab's channel, from the
    /// menu or the strip's offer (VS-109), or "Stop remembering".
    RememberActive(bool),
    /// The strip's offer to remember, dismissed (VS-208).
    DismissOffer,
}

/// What the active tab's channel knows of the viewer, for the strip's mark
/// (VS-114) and the menu's choice (VS-109).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Known {
    /// No key: the channel does not know this browser.
    #[default]
    Not,
    /// A visit key.
    Visit,
    /// A remembered key.
    Remembered,
}
