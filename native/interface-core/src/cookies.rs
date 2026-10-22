//! The cookies panel's view and requests (`cookies.md` section 4.1,
//! requirement 21.6): the trusted core lists channels and sites with their
//! counts and sizes, never a cookie's name or value, and the interface asks
//! for changes by the numbers the view gave (as history's, requirement
//! 20.5).

use alloc::string::String;
use alloc::vec::Vec;

pub use crate::library::{MAX_NOTICE, MAX_QUERY, MAX_VIEW_ROWS};

/// The longest label: a channel's identity, the address field's limit.
pub const MAX_LABEL: usize = crate::browser::MAX_ADDRESS;
/// The longest "last used" text, as "4 days ago".
pub const MAX_USED: usize = 32;

/// One row: a site, or a channel under it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CookieRow {
    /// The number it is asked for by.
    pub number: u32,
    /// A site (an origin) heading its channels, or a channel.
    pub site: bool,
    /// The origin, or the channel's folder on it.
    pub label: String,
    /// How many cookies.
    pub count: u32,
    /// Their bytes.
    pub bytes: u64,
    /// When last used, in words.
    pub used: String,
    /// The channel's cookies are blocked.
    pub blocked: bool,
}

/// What the panel shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CookiesView {
    /// The search the rows answer; empty for all.
    pub query: String,
    /// The place of the first row among all that answer.
    pub first: u32,
    /// How many rows answer.
    pub total: u32,
    /// At most [`MAX_VIEW_ROWS`].
    pub rows: Vec<CookieRow>,
    /// Cookies kept in all, and their bytes.
    pub count: u32,
    /// Their bytes.
    pub bytes: u64,
    /// What the panel says: why cookies are kept for this run only, the
    /// result of a deletion.
    pub notice: String,
}

/// What the panel asks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CookiesRequest {
    /// The panel opened or closed: the trusted core sends views while open.
    Panel(bool),
    /// The rows that answer `text`, from the `first`.
    Query {
        /// The search; empty for all.
        text: String,
        /// The first row wanted.
        first: u32,
    },
    /// Delete a row's cookies: a site's, or a channel's.
    Delete(u32),
    /// Block a channel's cookies, or unblock them.
    Block {
        /// The channel's row.
        number: u32,
        /// Blocked.
        blocked: bool,
    },
    /// Delete every cookie.
    DeleteAll,
}
