//! The bounds every message keeps: the largest payload, the page numbers,
//! the longest texts and the most tabs, and the window numbers that mean a
//! new window and the first.

/// The largest payload accepted, as the engine's pipe (`MAX_FRAME`).
pub const MAX_PAYLOAD: usize = 256 * 1024;
/// A channel has at most 2,048 page numbers: eight magazines of 256
/// (EN 300 706 clause 7.1.2).
pub const MAX_PAGES: usize = 8 * 256;
/// Bounds on the strip's and the view's text, in bytes of UTF-8: a
/// channel's title.
pub const MAX_TITLE: usize = 256;
/// A channel's identity, its host name.
pub const MAX_IDENTITY: usize = 256;
/// The longest notice the strip says, as "The proxy refused the
/// connection to" and a host of at most 253 bytes.
pub const MAX_STRIP_NOTICE: usize = 320;
/// The address bar's limit, as the trusted core's address rules.
pub const MAX_ADDRESS: usize = interface_core::browser::MAX_ADDRESS;
/// A page title: one row of a teletext page is 40 characters, and each
/// takes at most four bytes of UTF-8.
pub const MAX_PAGE_TITLE: usize = 40 * 4;
/// A bank's name.
pub const MAX_BANK_NAME: usize = 64;
/// A tab's title.
pub const MAX_TAB_TITLE: usize = 128;
/// More tabs than any window shows.
pub const MAX_TABS: usize = 64;

/// A window's number on the wire, for a new window: no window has it.
pub const NEW_WINDOW: u32 = 0;
/// The first window's number: the window the interface opens at start, and
/// the one messages are for until a `Target` names another.
pub const FIRST_WINDOW: u32 = 1;
